//! One admission queue for voxel CPU work. Pending futures never occupy a worker.
use bevy_app::{App, Plugin, PostUpdate};
use bevy_diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, RegisterDiagnostic};
use bevy_ecs::prelude::*;
const ACTIVE: DiagnosticPath = DiagnosticPath::const_new("voxel/cpu_active");
const PENDING: DiagnosticPath = DiagnosticPath::const_new("voxel/cpu_pending");
use bevy_tasks::{AsyncComputeTaskPool, Task};
use std::{
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

struct Pending {
    class: WorkClass,
    distance: u32,
    sequence: u64,
    queued: Instant,
    work: Pin<Box<dyn Future<Output = ()> + Send>>,
}

#[derive(Default)]
struct Queue {
    next: u64,
    pending: Vec<Pending>,
}

impl Queue {
    fn pop(&mut self) -> Option<Pending> {
        let now = Instant::now();
        let best = self
            .pending
            .iter()
            .enumerate()
            .min_by_key(|(_, job)| {
                // Promote one class every second; long-lived distant jobs eventually lead.
                let age = now.duration_since(job.queued).as_secs().min(16) as i32;
                (job.class as i32 - age, job.distance, job.sequence)
            })
            .map(|(i, _)| i)?;
        Some(self.pending.swap_remove(best))
    }
}

/// Shared across sessions: obsolete running jobs retain their slot until completion.
#[derive(Resource, Clone)]
pub struct VoxelTaskBudget {
    queue: Arc<Mutex<Queue>>,
    active: Arc<AtomicUsize>,
    workers: usize,
}

impl Default for VoxelTaskBudget {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        Self {
            queue: Arc::default(),
            active: Arc::default(),
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

impl VoxelTaskBudget {
    pub fn worker_count(&self) -> usize {
        self.workers
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
        queue.pending.push(Pending {
            class,
            distance,
            sequence,
            queued: Instant::now(),
            work: Box::pin(async move {
                if !send.is_closed() {
                    let result = work.await;
                    let _ = send.send(result).await;
                }
            }),
        });
        AsyncComputeTaskPool::get()
            .spawn(async move { receive.recv().await.expect("Voxel task result sender") })
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
        let Some(mut job) = queue.pop() else {
            break;
        };
        budget.active.fetch_add(1, Ordering::Relaxed);
        let slot = Slot(budget.active.clone());
        let budget = budget.clone();
        AsyncComputeTaskPool::get()
            .spawn(async move {
                let _slot = slot;
                loop {
                    job.work.await;
                    bevy_tasks::futures_lite::future::yield_now().await;
                    let next = budget.queue.lock().unwrap().pop();
                    let Some(next) = next else {
                        break;
                    };
                    // Refill on completion rather than leaving this CPU slot idle
                    // until the main thread reaches the next frame's dispatcher.
                    job = next;
                }
            })
            .detach();
    }
    diagnostics.add_measurement(&ACTIVE, || budget.active.load(Ordering::Relaxed) as f64);
    diagnostics.add_measurement(&PENDING, || queue.pending.len() as f64);
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
