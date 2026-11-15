//! One admission queue for voxel CPU work. Pending futures never occupy a worker.
//!
//! Background work (autosave encoding) only fills spare capacity: at most a third
//! of the workers, and one while foreground work is queued, so deferred saves
//! neither delay terrain near the player nor saturate cores frame threads share.
use bevy_app::{App, Plugin, PostUpdate};
use bevy_diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, RegisterDiagnostic};
use bevy_ecs::prelude::*;
const ACTIVE: DiagnosticPath = DiagnosticPath::const_new("voxel/cpu_active");
const PENDING: DiagnosticPath = DiagnosticPath::const_new("voxel/cpu_pending");
use bevy_tasks::{AsyncComputeTaskPool, Task};
use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkClass {
    PlayerCollision,
    Lighting,
    EditedMesh,
    Loading,
    Streaming,
    Encoding,
}

impl WorkClass {
    /// Deferrable work nobody is waiting on.
    fn is_background(self) -> bool {
        self == Self::Encoding
    }
}

struct Pending {
    class: WorkClass,
    distance: u32,
    sequence: u64,
    queued: Instant,
    age: u32,
    work: Pin<Box<dyn Future<Output = ()> + Send>>,
}

type Rank = (i32, u32, u64, usize);
#[derive(Default)]
struct Queue {
    next: u64,
    jobs: Vec<Option<Pending>>,
    free: Vec<usize>,
    foreground: BTreeSet<Rank>,
    background: BTreeSet<Rank>,
    promotions: BTreeSet<(Instant, usize)>,
}
impl Queue {
    fn ranks(&mut self, background: bool) -> &mut BTreeSet<Rank> {
        if background {
            &mut self.background
        } else {
            &mut self.foreground
        }
    }
    fn rank(job: &Pending, index: usize) -> Rank {
        (
            job.class as i32 - job.age as i32,
            job.distance,
            job.sequence,
            index,
        )
    }
    fn insert(&mut self, job: Pending) -> usize {
        let index = self.free.pop().unwrap_or_else(|| {
            self.jobs.push(None);
            self.jobs.len() - 1
        });
        self.ranks(job.class.is_background())
            .insert(Self::rank(&job, index));
        self.promotions
            .insert((job.queued + Duration::from_secs(1), index));
        self.jobs[index] = Some(job);
        index
    }
    fn remove(&mut self, index: usize) -> Option<Pending> {
        let job = self.jobs[index].take()?;
        self.ranks(job.class.is_background())
            .remove(&Self::rank(&job, index));
        if job.age < 16 {
            self.promotions.remove(&(
                job.queued + Duration::from_secs(u64::from(job.age + 1)),
                index,
            ));
        }
        self.free.push(index);
        Some(job)
    }
    fn pop(&mut self, background_running: usize, background_limit: usize) -> Option<Pending> {
        let now = Instant::now();
        while let Some(&(when, index)) = self.promotions.first() {
            if when > now {
                break;
            }
            self.promotions.pop_first();
            let job = self.jobs[index].as_mut().unwrap();
            let old = Self::rank(job, index);
            job.age = now.duration_since(job.queued).as_secs().min(16) as u32;
            let new = Self::rank(job, index);
            let background = job.class.is_background();
            if job.age < 16 {
                self.promotions.insert((
                    job.queued + Duration::from_secs(u64::from(job.age + 1)),
                    index,
                ));
            }
            let ranks = self.ranks(background);
            ranks.remove(&old);
            ranks.insert(new);
        }
        let foreground = self.foreground.first().copied();
        let limit = if foreground.is_some() {
            1
        } else {
            background_limit
        };
        let background = self
            .background
            .first()
            .copied()
            .filter(|_| background_running < limit);
        let rank = match (foreground, background) {
            (Some(a), Some(b)) => a.min(b),
            (a, b) => a.or(b)?,
        };
        self.remove(rank.3)
    }
    fn len(&self) -> usize {
        self.foreground.len() + self.background.len()
    }
}

// Dropping a waiting Task releases its captured work immediately. The sequence
// protects a recycled slot if completion and receiver cancellation race.
struct WaitingJob {
    queue: Arc<Mutex<Queue>>,
    index: usize,
    sequence: u64,
}
impl Drop for WaitingJob {
    fn drop(&mut self) {
        let removed = {
            let mut queue = self.queue.lock().unwrap();
            if queue
                .jobs
                .get(self.index)
                .and_then(Option::as_ref)
                .is_some_and(|job| job.sequence == self.sequence)
            {
                queue.remove(self.index)
            } else {
                None
            }
        };
        drop(removed);
    }
}

/// Shared across sessions: obsolete running jobs retain their slot until completion.
#[derive(Resource, Clone)]
pub struct VoxelTaskBudget {
    queue: Arc<Mutex<Queue>>,
    active: Arc<AtomicUsize>,
    /// Running background jobs; only incremented under the queue lock.
    background: Arc<AtomicUsize>,
    workers: usize,
}

impl Default for VoxelTaskBudget {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        Self {
            queue: Arc::default(),
            active: Arc::default(),
            background: Arc::default(),
            workers: AsyncComputeTaskPool::try_get()
                .map_or((cores / 2).clamp(1, 10), |pool| pool.thread_num().max(1)),
        }
    }
}

struct Slot(Arc<AtomicUsize>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

type Admitted = (Pin<Box<dyn Future<Output = ()> + Send>>, Option<Slot>);

impl VoxelTaskBudget {
    pub fn worker_count(&self) -> usize {
        self.workers
    }
    /// Pops the next admissible job; a background job holds its slot until dropped.
    fn admit(&self, queue: &mut Queue) -> Option<Admitted> {
        // A third of the workers keeps idle-time saves progressing without
        // occupying the cores that frame threads also need.
        let limit = (self.workers / 3).max(1);
        let job = queue.pop(self.background.load(Ordering::Relaxed), limit)?;
        let slot = job.class.is_background().then(|| {
            self.background.fetch_add(1, Ordering::Relaxed);
            Slot(self.background.clone())
        });
        Some((job.work, slot))
    }
    pub fn spawn<T: Send + 'static>(
        &self,
        class: WorkClass,
        distance: u32,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Task<T> {
        let (send, receive) = async_channel::bounded(1);
        let mut queue = self.queue.lock().unwrap();
        let sequence = queue.next;
        queue.next += 1;
        let index = queue.insert(Pending {
            class,
            distance,
            sequence,
            queued: Instant::now(),
            age: 0,
            work: Box::pin(async move {
                if !send.is_closed() {
                    let result = work.await;
                    let _ = send.send(result).await;
                }
            }),
        });
        let waiting = WaitingJob {
            queue: self.queue.clone(),
            index,
            sequence,
        };
        drop(queue);
        AsyncComputeTaskPool::get().spawn(async move {
            let _waiting = waiting;
            receive.recv().await.expect("Voxel task result sender")
        })
    }
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoxelTasks {
    Collect,
    Demand,
    Publish,
    Requests,
    Dispatch,
}

/// Expansion and dispatch share one millisecond of main-thread scheduling per frame.
#[derive(Resource)]
pub struct SchedulingDeadline {
    pub until: Instant,
    pub remaining: Duration,
}
impl Default for SchedulingDeadline {
    fn default() -> Self {
        Self {
            until: Instant::now(),
            remaining: Duration::ZERO,
        }
    }
}

pub(crate) fn begin_frame(mut deadline: ResMut<SchedulingDeadline>) {
    deadline.remaining = Duration::from_millis(1);
}

fn dispatch(
    budget: Res<VoxelTaskBudget>,
    deadline: Res<SchedulingDeadline>,
    mut diagnostics: Diagnostics,
) {
    let until = Instant::now() + deadline.remaining;
    let mut queue = budget.queue.lock().unwrap();
    while budget.active.load(Ordering::Relaxed) < budget.workers && Instant::now() < until {
        let Some(job) = budget.admit(&mut queue) else {
            break;
        };
        budget.active.fetch_add(1, Ordering::Relaxed);
        let slot = Slot(budget.active.clone());
        let budget = budget.clone();
        AsyncComputeTaskPool::get()
            .spawn(async move {
                let _slot = slot;
                let mut next = Some(job);
                while let Some((work, background)) = next {
                    work.await;
                    drop(background);
                    bevy_tasks::futures_lite::future::yield_now().await;
                    // Refill on completion rather than leaving this CPU slot idle
                    // until the main thread reaches the next frame's dispatcher.
                    next = budget.admit(&mut budget.queue.lock().unwrap());
                }
            })
            .detach();
    }
    diagnostics.add_measurement(&ACTIVE, || budget.active.load(Ordering::Relaxed) as f64);
    diagnostics.add_measurement(&PENDING, || queue.len() as f64);
}

pub fn configure(app: &mut App) {
    app.register_diagnostic(Diagnostic::new(ACTIVE))
        .register_diagnostic(Diagnostic::new(PENDING))
        .init_resource::<VoxelTaskBudget>()
        .init_resource::<SchedulingDeadline>()
        .configure_sets(
            PostUpdate,
            (
                VoxelTasks::Collect,
                VoxelTasks::Demand,
                VoxelTasks::Publish,
                VoxelTasks::Requests,
                VoxelTasks::Dispatch,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            begin_frame
                .after(VoxelTasks::Collect)
                .before(VoxelTasks::Demand),
        )
        .add_systems(PostUpdate, dispatch.in_set(VoxelTasks::Dispatch));
}

pub struct VoxelTaskPlugin;
impl Plugin for VoxelTaskPlugin {
    fn build(&self, app: &mut App) {
        configure(app);
    }
}
