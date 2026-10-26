//! Bevy owns generation requests, spatial dependencies, reservations and tasks.
use crate::lighting::LightingEngineConfig;
use crate::{
    ChunkSnapshot, ChunkStorage, Dimension, ProtoChunk, TerrainQuality, VanillaBlockRegistry,
    chunk_system::generation_cache::SurfaceBiomeNeighborhood,
    chunk_system::{Cache, Chunk, ChunkPos, StagedChunkEnum as Stage},
    generation::generator::VanillaGenerator,
    tasks::{GenerationSystems, SchedulingDeadline, VoxelTaskBudget, WorkClass},
};
use bevy_app::{App, Plugin, PostUpdate};
use bevy_ecs::prelude::*;
use bevy_math::IVec2;
use bevy_tasks::{IoTaskPool, Task, futures::check_ready};
use pumpkin_util::world_seed::Seed;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Resource, Clone)]
pub struct GenerationSettings {
    pub seed: i64,
    pub dimension: Dimension,
    pub quality: TerrainQuality,
    pub version: &'static str,
}

/// The game retains this column while its presentation or gameplay data is loaded.
#[derive(Component)]
pub struct ChunkRetention;

#[derive(Message)]
pub struct ChunkAvailable {
    pub session: u64,
    pub position: IVec2,
    pub revision: u64,
}
struct Request {
    position: ChunkPos,
    send: async_channel::Sender<Result<ChunkSnapshot, String>>,
}
struct Shared {
    id: u64,
    settings: GenerationSettings,
    generator: std::sync::OnceLock<Result<Arc<VanillaGenerator>, String>>,
    initialization_waiters: Mutex<Vec<async_channel::Sender<Result<(), String>>>>,
    storage: Option<Arc<dyn ChunkStorage>>,
    cancelled: AtomicBool,
    view: Mutex<Option<(IVec2, u8)>>,
    requests: Mutex<Vec<Request>>,
    snapshots: Mutex<HashMap<ChunkPos, ChunkSnapshot>>,
    entities: Mutex<HashMap<ChunkPos, Entity>>,
    flush: Mutex<Vec<async_channel::Sender<Result<(), String>>>>,
    paused: AtomicBool,
    loading: AtomicBool,
    errors: Mutex<Vec<String>>,
}

/// Immutable session handle. Procedural data is never replaced by player edits.
#[derive(Resource, Clone)]
pub struct GenerationSession(Arc<Shared>);
impl GenerationSession {
    pub fn new(
        seed: i64,
        dimension: Dimension,
        quality: TerrainQuality,
        storage: Option<Arc<dyn ChunkStorage>>,
    ) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(Arc::new(Shared {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            settings: GenerationSettings {
                seed,
                dimension,
                quality,
                version: crate::GENERATOR_VERSION,
            },
            generator: std::sync::OnceLock::new(),
            initialization_waiters: Mutex::default(),
            storage,
            cancelled: AtomicBool::new(false),
            view: Mutex::default(),
            requests: Mutex::default(),
            snapshots: Mutex::default(),
            entities: Mutex::default(),
            flush: Mutex::default(),
            paused: AtomicBool::new(false),
            loading: AtomicBool::new(true),
            errors: Mutex::default(),
        }))
    }
    fn generator(&self) -> Arc<VanillaGenerator> {
        self.0
            .generator
            .get()
            .expect("Generation initialization completed")
            .as_ref()
            .expect("Generation initialization succeeded")
            .clone()
    }
    fn work_class(&self) -> WorkClass {
        if self.0.loading.load(Ordering::Relaxed) {
            WorkClass::Loading
        } else {
            WorkClass::Streaming
        }
    }
    pub async fn ready(&self) -> io::Result<()> {
        let receive = {
            let mut waiters = self.0.initialization_waiters.lock().unwrap();
            if let Some(result) = self.0.generator.get() {
                return result
                    .as_ref()
                    .map(|_| ())
                    .map_err(|error| io::Error::other(error.clone()));
            }
            if self.is_cancelled() {
                return Err(io::Error::other("World request cancelled"));
            }
            let (send, receive) = async_channel::bounded(1);
            waiters.push(send);
            receive
        };
        receive
            .recv()
            .await
            .map_err(io::Error::other)?
            .map_err(io::Error::other)
    }
    pub fn id(&self) -> u64 {
        self.0.id
    }
    pub fn settings(&self) -> &GenerationSettings {
        &self.0.settings
    }
    pub fn set_view(&self, center: IVec2, radius: u8) {
        *self.0.view.lock().unwrap() = Some((center, radius));
    }
    pub fn clear_view(&self) {
        *self.0.view.lock().unwrap() = None;
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
        self.clear_view();
        for waiter in self.0.initialization_waiters.lock().unwrap().drain(..) {
            let _ = waiter.try_send(Err("World request cancelled".into()));
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Relaxed)
    }
    pub fn entity(&self, position: IVec2) -> Option<Entity> {
        self.0
            .entities
            .lock()
            .unwrap()
            .get(&ChunkPos::new(position.x, position.y))
            .copied()
    }
    pub fn snapshot(&self, position: IVec2) -> Option<ChunkSnapshot> {
        self.0
            .snapshots
            .lock()
            .unwrap()
            .get(&ChunkPos::new(position.x, position.y))
            .cloned()
    }
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.errors.lock().unwrap())
    }
    pub async fn request_chunk(&self, position: IVec2) -> io::Result<ChunkSnapshot> {
        if self.is_cancelled() {
            return Err(io::Error::other("World request cancelled"));
        }
        if let Some(snapshot) = self.snapshot(position) {
            return Ok(snapshot);
        }
        let (send, receive) = async_channel::bounded(1);
        self.0.requests.lock().unwrap().push(Request {
            position: ChunkPos::new(position.x, position.y),
            send,
        });
        receive
            .recv()
            .await
            .map_err(io::Error::other)?
            .map_err(io::Error::other)
    }
    /// Await `ready` before calling this compute-only probe.
    /// Terrain-only probes retain the existing calculation order and neighbor biome sampling.
    pub fn probe_terrain(&self, position: IVec2) -> ChunkSnapshot {
        let chunk = crate::chunk_system::generate_single_chunk(
            &self.generator(),
            &VanillaBlockRegistry,
            position.x,
            position.y,
            Stage::Surface,
        );
        let Chunk::Proto(chunk) = chunk else {
            unreachable!()
        };
        ChunkSnapshot::from_proto(
            &chunk,
            self.settings().dimension.pumpkin().min_y,
            self.settings().dimension.pumpkin().height,
        )
    }
    pub fn pause(&self) {
        self.0.paused.store(true, Ordering::Relaxed);
    }
    pub fn resume(&self) {
        self.0.loading.store(false, Ordering::Relaxed);
        self.0.paused.store(false, Ordering::Relaxed);
    }
    /// The ECS runtime drains active stages, snapshots every retained stage, then
    /// acknowledges the database flush without blocking the main thread.
    pub async fn flush(&self) -> io::Result<()> {
        if self.is_cancelled() {
            return Err(io::Error::other("World request cancelled"));
        }
        let (send, receive) = async_channel::bounded(1);
        self.0.flush.lock().unwrap().push(send);
        receive
            .recv()
            .await
            .map_err(io::Error::other)?
            .map_err(io::Error::other)
    }
}

struct Entry {
    entity: Entity,
    references: [u32; Stage::COUNT],
    chunk: Option<Chunk>,
    stage: Stage,
    revision: u64,
    saved_revision: Option<u64>,
    readers: u32,
    writing: bool,
    failed: bool,
    queued: bool,
}
impl Entry {
    fn target(&self) -> Stage {
        (1..Stage::COUNT)
            .rev()
            .find(|&s| self.references[s] > 0)
            .map_or(Stage::None, |s| Stage::from(s as u8))
    }
}
struct FlushCapture {
    remaining: usize,
    chunks: Vec<(ChunkPos, Chunk)>,
    writes: Vec<(ChunkPos, u64)>,
    replies: Vec<async_channel::Sender<Result<(), String>>>,
}
struct Runtime {
    session: GenerationSession,
    entries: HashMap<ChunkPos, Entry>,
    roots: HashSet<ChunkPos>,
    root_order: VecDeque<ChunkPos>,
    roots_to_scan: usize,
    view_ring: i32,
    view_edge: i32,
    view: Option<(IVec2, u8)>,
    changes: VecDeque<(ChunkPos, Stage, bool)>,
    candidates: VecDeque<ChunkPos>,
    ready: BinaryHeap<Reverse<(u32, i32, i32)>>,
    requests: Vec<Request>,
    active: usize,
    trim_pending: Option<(IVec2, u8)>,
    trim_in_flight: bool,
    flush_capture: Option<FlushCapture>,
}
impl Runtime {
    fn new(session: GenerationSession) -> Self {
        Self {
            session,
            entries: HashMap::new(),
            roots: HashSet::new(),
            root_order: VecDeque::new(),
            roots_to_scan: 0,
            view_ring: 0,
            view_edge: 0,
            view: None,
            changes: VecDeque::new(),
            candidates: VecDeque::new(),
            ready: BinaryHeap::new(),
            requests: Vec::new(),
            active: 0,
            trim_pending: None,
            trim_in_flight: false,
            flush_capture: None,
        }
    }

    fn queue(&mut self, position: ChunkPos) {
        let Some(entry) = self.entries.get_mut(&position) else {
            return;
        };
        if !entry.queued {
            entry.queued = true;
            let center = self.view.map_or(IVec2::ZERO, |(center, _)| center);
            let distance = crate::coordinates::chunk_distance_squared(
                IVec2::new(position.x, position.y),
                center,
            ) as u32;
            self.ready.push(Reverse((distance, position.x, position.y)));
        }
    }

    fn spawn(
        &mut self,
        commands: &mut Commands,
        position: ChunkPos,
        writes: Vec<(ChunkPos, u64)>,
        reads: Vec<(ChunkPos, u64)>,
        task: Task<Output>,
    ) {
        self.active += 1;
        commands.spawn(GenerationTask {
            session: self.session.id(),
            position,
            writes,
            reads,
            task,
        });
    }
}
#[derive(Resource, Default)]
struct GenerationGraph(Vec<Runtime>);

enum Output {
    Trimmed,
    Initialized(Result<Arc<VanillaGenerator>, String>),
    Generated(Result<(Cache, Option<ChunkSnapshot>, bool), String>),
    Stored {
        chunks: Vec<(ChunkPos, Chunk)>,
        result: Result<(), String>,
        replies: Vec<async_channel::Sender<Result<(), String>>>,
    },
}
#[derive(Component)]
pub struct GenerationTask {
    session: u64,
    position: ChunkPos,
    writes: Vec<(ChunkPos, u64)>,
    reads: Vec<(ChunkPos, u64)>,
    task: Task<Output>,
}

pub struct WorldGenerationPlugin;
impl Plugin for WorldGenerationPlugin {
    fn build(&self, app: &mut App) {
        crate::tasks::configure(app);
        app.init_resource::<GenerationGraph>()
            .add_message::<ChunkAvailable>()
            .add_systems(PostUpdate, collect.in_set(GenerationSystems::Collect))
            .add_systems(PostUpdate, update_demands.in_set(GenerationSystems::Demand))
            .add_systems(PostUpdate, schedule.in_set(GenerationSystems::Requests));
    }
}

fn dependencies(position: ChunkPos, stage: Stage, mut visit: impl FnMut(ChunkPos, Stage)) {
    if stage <= Stage::Empty {
        return;
    }
    let dependencies = stage.get_direct_dependencies();
    let radius = stage.get_direct_radius();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let distance = dx.abs().max(dz.abs()) as usize;
            visit(position.add_raw(dx, dz), dependencies[distance]);
        }
    }
}

fn update_demands(
    mut commands: Commands,
    session: Option<Res<GenerationSession>>,
    mut graph: ResMut<GenerationGraph>,
    mut deadline: ResMut<SchedulingDeadline>,
) {
    let started = Instant::now();
    deadline.until = started + deadline.remaining / 2;
    if let Some(session) = session
        && !graph
            .0
            .iter()
            .any(|runtime| runtime.session.id() == session.id())
    {
        for runtime in &graph.0 {
            runtime.session.cancel();
        }
        commands.insert_resource(session.settings().clone());
        graph.0.push(Runtime::new(session.clone()));
    }
    for runtime in &mut graph.0 {
        let view = *runtime.session.0.view.lock().unwrap();
        if runtime.view != view {
            runtime.view = view;
            runtime.trim_pending = view;
            runtime.roots_to_scan = runtime.root_order.len();
            runtime.view_ring = 0;
            runtime.view_edge = 0;
            let queued = std::mem::take(&mut runtime.ready);
            for Reverse((_, x, z)) in queued {
                let position = ChunkPos::new(x, z);
                if let Some(entry) = runtime.entries.get_mut(&position) {
                    entry.queued = false;
                    runtime.queue(position);
                }
            }
        }
        while Instant::now() < deadline.until {
            if runtime.roots_to_scan > 0 {
                runtime.roots_to_scan -= 1;
                let position = runtime.root_order.pop_front().unwrap();
                let keep = view.is_some_and(|(center, radius)| {
                    crate::coordinates::chunk_distance_squared(
                        IVec2::new(position.x, position.y),
                        center,
                    ) <= i32::from(radius).pow(2)
                });
                if keep {
                    runtime.root_order.push_back(position);
                } else {
                    runtime.roots.remove(&position);
                    runtime.changes.push_back((position, Stage::Full, false));
                }
            } else if let Some((center, radius)) = view
                && runtime.view_ring <= i32::from(radius)
            {
                let r = runtime.view_ring;
                let t = runtime.view_edge;
                let (dx, dz) = if r == 0 {
                    (0, 0)
                } else if t < 2 * r {
                    (-r + t, -r)
                } else if t < 4 * r {
                    (r, -r + t - 2 * r)
                } else if t < 6 * r {
                    (r - t + 4 * r, r)
                } else {
                    (-r, r - t + 6 * r)
                };
                runtime.view_edge += 1;
                if r == 0 || runtime.view_edge == 8 * r {
                    runtime.view_ring += 1;
                    runtime.view_edge = 0;
                }
                if dx * dx + dz * dz > i32::from(radius).pow(2) {
                    continue;
                }
                let position = ChunkPos::new(center.x + dx, center.y + dz);
                if runtime.roots.insert(position) {
                    runtime.root_order.push_back(position);
                    runtime.changes.push_back((position, Stage::Full, true));
                }
            } else {
                break;
            }
        }
        let requests = std::mem::take(&mut *runtime.session.0.requests.lock().unwrap());
        for request in requests {
            runtime
                .changes
                .push_back((request.position, Stage::Full, true));
            runtime.requests.push(request);
        }
        let snapshots = runtime.session.0.snapshots.lock().unwrap();
        runtime.requests.retain(|request| {
            let reply = if runtime.session.is_cancelled() {
                Some(Err("World request cancelled".to_owned()))
            } else {
                snapshots.get(&request.position).cloned().map(Ok)
            };
            if request.send.is_closed() || reply.is_some() {
                if let Some(reply) = reply {
                    let _ = request.send.try_send(reply);
                }
                runtime
                    .changes
                    .push_back((request.position, Stage::Full, false));
                false
            } else {
                true
            }
        });
        drop(snapshots);
        while Instant::now() < deadline.until {
            let Some((position, stage, add)) = runtime.changes.pop_front() else {
                break;
            };
            let entry = runtime.entries.entry(position).or_insert_with(|| {
                let entity = commands.spawn_empty().id();
                runtime
                    .session
                    .0
                    .entities
                    .lock()
                    .unwrap()
                    .insert(position, entity);
                runtime.candidates.push_back(position);
                Entry {
                    entity,
                    references: [0; Stage::COUNT],
                    chunk: None,
                    stage: Stage::None,
                    revision: 0,
                    saved_revision: None,
                    readers: 0,
                    writing: false,
                    failed: false,
                    queued: false,
                }
            });
            let count = &mut entry.references[stage as usize];
            let propagate = if add {
                *count += 1;
                *count == 1
            } else {
                *count = count
                    .checked_sub(1)
                    .expect("Generation dependency reference");
                *count == 0
            };
            if propagate {
                dependencies(position, stage, |p, s| {
                    runtime.changes.push_back((p, s, add))
                });
            }
            runtime.queue(position);
        }
    }
    deadline.remaining = deadline.remaining.saturating_sub(started.elapsed());
}

fn collect(
    mut commands: Commands,
    mut graph: ResMut<GenerationGraph>,
    mut tasks: Query<(Entity, &mut GenerationTask)>,
    mut available: MessageWriter<ChunkAvailable>,
) {
    for (entity, mut task) in &mut tasks {
        let Some(output) = check_ready(&mut task.task) else {
            continue;
        };
        commands.entity(entity).despawn();
        let Some(runtime) = graph
            .0
            .iter_mut()
            .find(|runtime| runtime.session.id() == task.session)
        else {
            continue;
        };
        runtime.active -= 1;
        let valid = task
            .writes
            .iter()
            .chain(task.reads.iter())
            .all(|(p, revision)| {
                runtime
                    .entries
                    .get(p)
                    .is_some_and(|entry| entry.revision == *revision)
            });
        for (p, _) in &task.reads {
            runtime.entries.get_mut(p).unwrap().readers -= 1;
        }
        for (p, _) in &task.writes {
            runtime.entries.get_mut(p).unwrap().writing = false;
        }
        if runtime.session.is_cancelled() {
            IoTaskPool::get()
                .spawn(async move {
                    drop(output);
                })
                .detach();
            continue;
        }
        let result = match output {
            Output::Trimmed => {
                runtime.trim_in_flight = false;
                Ok(())
            }
            Output::Initialized(result) => {
                let notification = result.as_ref().map(|_| ()).map_err(Clone::clone);
                assert!(
                    runtime.session.0.generator.set(result).is_ok(),
                    "Generation initialized once per session"
                );
                for waiter in runtime
                    .session
                    .0
                    .initialization_waiters
                    .lock()
                    .unwrap()
                    .drain(..)
                {
                    let _ = waiter.try_send(notification.clone());
                }
                notification
            }
            Output::Stored {
                chunks,
                result,
                replies,
            } => {
                for (position, chunk) in chunks {
                    let entry = runtime.entries.get_mut(&position).unwrap();
                    entry.chunk = Some(chunk);
                    if result.is_ok() {
                        entry.saved_revision = Some(entry.revision);
                    }
                }
                for reply in replies {
                    let _ = reply.try_send(result.clone());
                }
                result
            }
            Output::Generated(result) => result.map(|(cache, snapshot, saved)| {
                assert!(
                    valid,
                    "Generation reservations must preserve all read revisions"
                );
                for (i, chunk) in cache.chunks.into_iter().enumerate() {
                    let position = ChunkPos::new(
                        cache.x + i as i32 / cache.size,
                        cache.z + i as i32 % cache.size,
                    );
                    let entry = runtime.entries.get_mut(&position).unwrap();
                    entry.stage = Stage::from(chunk.get_stage_id());
                    entry.revision += 1;
                    if saved {
                        entry.saved_revision = Some(entry.revision);
                    }
                    entry.chunk = Some(chunk);
                }
                if let Some(snapshot) = snapshot {
                    runtime
                        .session
                        .0
                        .snapshots
                        .lock()
                        .unwrap()
                        .insert(task.position, snapshot);
                    available.write(ChunkAvailable {
                        session: task.session,
                        position: IVec2::new(task.position.x, task.position.y),
                        revision: runtime.entries[&task.position].revision,
                    });
                }
            }),
        };
        // Only completions and demand changes wake stage admission. A blocked
        // chunk waits for its neighbors instead of scanning the whole view again.
        for (position, _) in task.writes.iter().chain(task.reads.iter()) {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    runtime.queue(position.add_raw(dx, dz));
                }
            }
        }
        if let Err(error) = result {
            if let Some(entry) = runtime.entries.get_mut(&task.position) {
                entry.failed = true;
            }
            runtime.session.pause();
            runtime.session.0.errors.lock().unwrap().push(error);
        }
    }
    let mut i = 0;
    while i < graph.0.len() {
        if graph.0[i].session.is_cancelled() && graph.0[i].active == 0 {
            let runtime = graph.0.swap_remove(i);
            for entry in runtime.entries.values() {
                commands.entity(entry.entity).despawn();
            }
            for reply in runtime.session.0.flush.lock().unwrap().drain(..) {
                let _ = reply.try_send(Err("World request cancelled".into()));
            }
            IoTaskPool::get()
                .spawn(async move {
                    drop(runtime);
                })
                .detach();
        } else {
            i += 1;
        }
    }
}

fn schedule(
    mut commands: Commands,
    mut graph: ResMut<GenerationGraph>,
    budget: Res<VoxelTaskBudget>,
    retained: Query<(), With<ChunkRetention>>,
    mut deadline: ResMut<SchedulingDeadline>,
) {
    let started = Instant::now();
    deadline.until = started
        + deadline
            .remaining
            .saturating_sub(std::time::Duration::from_micros(100));
    for runtime in &mut graph.0 {
        if Instant::now() >= deadline.until {
            break;
        }
        if runtime.session.is_cancelled() {
            continue;
        }
        if runtime.session.0.generator.get().is_none() {
            if runtime.active == 0 {
                let settings = runtime.session.settings().clone();
                let task = budget.spawn(WorkClass::Loading, 0, async move {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut generator = VanillaGenerator::new(
                            Seed(settings.seed as u64),
                            settings.dimension.pumpkin(),
                        );
                        generator.fast_terrain = settings.quality == TerrainQuality::Fast;
                        Arc::new(generator)
                    }))
                    .map_err(|_| "Generation initialization failed".to_owned());
                    Output::Initialized(result)
                });
                runtime.spawn(
                    &mut commands,
                    ChunkPos::new(0, 0),
                    Vec::new(),
                    Vec::new(),
                    task,
                );
            }
            continue;
        }
        if runtime
            .session
            .0
            .generator
            .get()
            .is_some_and(Result::is_err)
        {
            continue;
        }
        if runtime.flush_capture.is_some() || !runtime.session.0.flush.lock().unwrap().is_empty() {
            if runtime.active > 0 {
                continue;
            }
            let capture = runtime.flush_capture.get_or_insert_with(|| FlushCapture {
                remaining: runtime.candidates.len(),
                chunks: Vec::new(),
                writes: Vec::new(),
                replies: std::mem::take(&mut *runtime.session.0.flush.lock().unwrap()),
            });
            while capture.remaining > 0 && Instant::now() < deadline.until {
                capture.remaining -= 1;
                let position = runtime.candidates.pop_front().unwrap();
                let Some(entry) = runtime.entries.get_mut(&position) else {
                    continue;
                };
                runtime.candidates.push_back(position);
                if let Some(chunk) = entry.chunk.take() {
                    entry.writing = true;
                    capture.writes.push((position, entry.revision));
                    capture.chunks.push((position, chunk));
                }
            }
            if capture.remaining == 0 {
                let capture = runtime.flush_capture.take().unwrap();
                spawn_store(
                    runtime,
                    capture.chunks,
                    capture.writes,
                    capture.replies,
                    &budget,
                    &mut commands,
                );
            }
            continue;
        }
        if runtime.session.0.paused.load(Ordering::Relaxed) {
            continue;
        }
        if !runtime.trim_in_flight
            && runtime.active < budget.worker_count()
            && let Some((center, radius)) = runtime.trim_pending.take()
        {
            let session = runtime.session.clone();
            let task = budget.spawn(WorkClass::Streaming, 0, async move {
                if !session.is_cancelled() {
                    session
                        .generator()
                        .global_structure_cache
                        .trim_structure_starts(center.x, center.y, i32::from(radius.min(32)) + 32);
                }
                Output::Trimmed
            });
            runtime.trim_in_flight = true;
            runtime.spawn(
                &mut commands,
                ChunkPos::new(center.x, center.y),
                Vec::new(),
                Vec::new(),
                task,
            );
        }
        // Incremental eviction also observes presentation retention being removed.
        let mut evicted = Vec::new();
        let mut evicted_writes = Vec::new();
        for _ in 0..runtime.candidates.len().min(32) {
            if Instant::now() >= deadline.until {
                break;
            }
            let Some(position) = runtime.candidates.pop_front() else {
                break;
            };
            if !runtime.entries.contains_key(&position) {
                continue;
            }
            runtime.candidates.push_back(position);
            if runtime.changes.is_empty() && runtime.entries[&position].target() == Stage::None {
                let entry = runtime.entries.get_mut(&position).unwrap();
                if !entry.writing && entry.readers == 0 && !retained.contains(entry.entity) {
                    if entry.chunk.is_some()
                        && runtime.session.0.storage.is_some()
                        && entry.saved_revision != Some(entry.revision)
                    {
                        if runtime.active < budget.worker_count() {
                            entry.writing = true;
                            evicted_writes.push((position, entry.revision));
                            evicted.push((position, entry.chunk.take().unwrap()));
                        }
                    } else {
                        let entry = runtime.entries.remove(&position).unwrap();
                        runtime.candidates.pop_back();
                        runtime.session.0.entities.lock().unwrap().remove(&position);
                        commands.entity(entry.entity).despawn();
                        runtime
                            .session
                            .0
                            .snapshots
                            .lock()
                            .unwrap()
                            .remove(&position);
                        IoTaskPool::get()
                            .spawn(async move {
                                drop(entry);
                            })
                            .detach();
                    }
                }
            }
        }
        if !evicted.is_empty() {
            spawn_store(
                runtime,
                evicted,
                evicted_writes,
                Vec::new(),
                &budget,
                &mut commands,
            );
        }
        // I/O and completed results do not occupy CPU workers. Keep a bounded
        // second wave admitted so workers can refill between Bevy frames.
        while Instant::now() < deadline.until && runtime.active < budget.worker_count() * 2 {
            let Some(Reverse((distance, x, z))) = runtime.ready.pop() else {
                break;
            };
            let position = ChunkPos::new(x, z);
            let Some(entry) = runtime.entries.get_mut(&position) else {
                continue;
            };
            if !std::mem::take(&mut entry.queued) {
                continue;
            }
            let entry = &runtime.entries[&position];
            if entry.failed || entry.writing || entry.target() <= entry.stage {
                continue;
            }
            if entry.stage == Stage::None {
                let target = entry.target();
                let session = runtime.session.clone();
                let generator_session = session.clone();
                let decode_budget = budget.clone();
                let task = IoTaskPool::get().spawn(async move {
                    let result = (|| -> Result<_, String> {
                        if session.is_cancelled() {
                            return Err("World request cancelled".into());
                        }
                        let stored = session
                            .0
                            .storage
                            .as_ref()
                            .map(|storage| storage.read_chunks(&[(position.x, position.y)]))
                            .transpose()
                            .map_err(|error| error.to_string())?;
                        if stored.as_ref().is_some_and(|chunks| chunks.len() != 1) {
                            return Err("Invalid generation storage response".into());
                        }
                        Ok(stored.and_then(|mut chunks| chunks.pop()).flatten())
                    })();
                    decode_budget
                        .spawn(generator_session.work_class(), distance, async move {
                            generate(
                                &generator_session,
                                position,
                                |generator| {
                                    let record = result?;
                                    let saved = record.is_some();
                                    let chunk = if let Some(record) = record {
                                        crate::codec::decode_generated(record, position, generator)
                                            .map_err(|error| error.to_string())?
                                    } else {
                                        Chunk::Proto(Box::new(ProtoChunk::new(
                                            position.x, position.y, generator,
                                        )))
                                    };
                                    let mut cache = Cache::new(position.x, position.y, 1);
                                    cache.chunks.push(chunk);
                                    Ok((cache, saved))
                                },
                                None,
                                target,
                                0,
                            )
                        })
                        .await
                });
                let revision = entry.revision;
                runtime.entries.get_mut(&position).unwrap().writing = true;
                runtime.spawn(
                    &mut commands,
                    position,
                    vec![(position, revision)],
                    Vec::new(),
                    task,
                );
                continue;
            }
            let stage = Stage::from(entry.stage as u8 + 1);
            let target = entry.target();
            let mut ready = true;
            dependencies(position, stage, |p, s| {
                ready &= runtime
                    .entries
                    .get(&p)
                    .is_some_and(|entry| entry.stage >= s && !entry.writing && !entry.failed)
            });
            let read_radius = stage.get_read_radius();
            let write_radius = stage.get_write_radius();
            for dx in -read_radius..=read_radius {
                for dz in -read_radius..=read_radius {
                    ready &= runtime
                        .entries
                        .get(&position.add_raw(dx, dz))
                        .is_some_and(|entry| {
                            entry.chunk.is_some()
                                && !entry.writing
                                && (dx.abs().max(dz.abs()) > write_radius || entry.readers == 0)
                        });
                }
            }
            if !ready {
                continue;
            }
            let mut cache = Cache::new(
                position.x - write_radius,
                position.y - write_radius,
                write_radius * 2 + 1,
            );
            if stage == Stage::Surface {
                let mut biomes = SurfaceBiomeNeighborhood::new(position.x, position.y);
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        assert!(
                            biomes.push_chunk(
                                runtime.entries[&position.add_raw(dx, dz)]
                                    .chunk
                                    .as_ref()
                                    .unwrap()
                            )
                        );
                    }
                }
                cache.set_surface_biomes(biomes);
            }
            let mut reads = Vec::new();
            let mut writes = Vec::new();
            for dx in -read_radius..=read_radius {
                for dz in -read_radius..=read_radius {
                    let p = position.add_raw(dx, dz);
                    let entry = runtime.entries.get_mut(&p).unwrap();
                    if dx.abs().max(dz.abs()) <= write_radius {
                        entry.writing = true;
                        writes.push((p, entry.revision));
                        cache.chunks.push(entry.chunk.take().unwrap());
                    } else {
                        entry.readers += 1;
                        reads.push((p, entry.revision));
                    }
                }
            }
            let session = runtime.session.clone();
            let task = budget.spawn(session.work_class(), distance, async move {
                generate(
                    &session,
                    position,
                    |_| Ok((cache, false)),
                    Some(stage),
                    target,
                    read_radius,
                )
            });
            runtime.spawn(&mut commands, position, writes, reads, task);
        }
    }
    deadline.remaining = deadline.remaining.saturating_sub(started.elapsed());
}

fn generate(
    session: &GenerationSession,
    position: ChunkPos,
    prepare: impl FnOnce(&VanillaGenerator) -> Result<(Cache, bool), String>,
    first: Option<Stage>,
    target: Stage,
    read_radius: i32,
) -> Output {
    let mut stage = first.unwrap_or(Stage::Empty);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let generator = session.generator();
        let (mut cache, saved) = prepare(&generator)?;
        let center = (cache.size * cache.size / 2) as usize;
        let stored_stage = cache.chunks[center].get_stage_id();
        stage = first.unwrap_or_else(|| Stage::from(cache.chunks[center].get_stage_id()));
        while !session.is_cancelled() {
            cache.advance(
                stage,
                &generator,
                &VanillaBlockRegistry,
                &LightingEngineConfig::Default,
            );
            if stage >= target {
                break;
            }
            let next = Stage::from(stage as u8 + 1);
            // Continue only inside the cache and reservations granted by Bevy.
            // Additional neighbor requirements return ownership to the graph first.
            if next.get_read_radius() > read_radius || next.get_write_radius() > cache.size / 2 {
                break;
            }
            let mut ready = true;
            dependencies(position, next, |p, required| {
                let x = p.x - cache.x;
                let z = p.y - cache.z;
                ready &= x >= 0
                    && z >= 0
                    && x < cache.size
                    && z < cache.size
                    && cache.chunks[(x * cache.size + z) as usize].get_stage_id() >= required as u8;
            });
            if !ready {
                break;
            }
            stage = next;
        }
        let snapshot = match &cache.chunks[center] {
            Chunk::Complete(data) if !session.is_cancelled() => Some(data.snapshot()),
            _ => None,
        };
        let saved = saved && cache.chunks[center].get_stage_id() == stored_stage;
        Ok((cache, snapshot, saved))
    }))
    .unwrap_or_else(|_| {
        Err(format!(
            "Generation failed at ({}, {}) during {stage:?}",
            position.x, position.y
        ))
    });
    Output::Generated(result)
}

fn spawn_store(
    runtime: &mut Runtime,
    chunks: Vec<(ChunkPos, Chunk)>,
    writes: Vec<(ChunkPos, u64)>,
    replies: Vec<async_channel::Sender<Result<(), String>>>,
    budget: &VoxelTaskBudget,
    commands: &mut Commands,
) {
    let position = writes.first().map_or(ChunkPos::new(0, 0), |(p, _)| *p);
    let session = runtime.session.clone();
    let budget = budget.clone();
    let task = IoTaskPool::get().spawn(async move {
        let (chunks, records) = budget
            .spawn(WorkClass::Encoding, 0, async move {
                let records = chunks
                    .iter()
                    .map(|(p, chunk)| {
                        crate::codec::encode_generated(chunk).map(|record| ((p.x, p.y), record))
                    })
                    .collect::<io::Result<Vec<_>>>();
                (chunks, records)
            })
            .await;
        let result = records
            .and_then(|records| {
                if let Some(storage) = &session.0.storage {
                    storage.write_chunks(records)?;
                    storage.flush()?;
                }
                Ok(())
            })
            .map_err(|error| error.to_string());
        Output::Stored {
            chunks,
            result,
            replies,
        }
    });
    runtime.spawn(commands, position, writes, Vec::new(), task);
}
