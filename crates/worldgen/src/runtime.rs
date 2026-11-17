//! Bevy owns generation requests, spatial dependencies, reservations and tasks.
use crate::lighting::LightingEngineConfig;
use crate::{
    ChunkSnapshot, ChunkStorage, Dimension, ProtoChunk, VanillaBlockRegistry,
    chunk_system::generation_cache::SurfaceBiomeNeighborhood,
    chunk_system::{Cache, Chunk, ChunkPos, StagedChunkEnum as Stage},
    generation::generator::TerrainGenerator,
};
use bevy_app::{App, Plugin, PostUpdate};
use bevy_ecs::prelude::*;
use bevy_math::IVec2;
use bevy_tasks::{IoTaskPool, Task, futures::check_ready};
use hollow_engine::tasks::{SchedulingDeadline, VoxelTaskBudget, VoxelTasks, WorkClass};
use pumpkin_util::world_seed::Seed;
use rustc_hash::{FxHashMap, FxHashSet};
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
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
#[derive(Clone)]
struct ViewDemand {
    center: IVec2,
    radius: u8,
    cells: Arc<[IVec2]>,
    admission: Arc<[u32]>,
}
impl ViewDemand {
    fn contains(&self, position: IVec2) -> bool {
        self.cells
            .binary_search_by_key(&(position.x, position.y), |p| (p.x, p.y))
            .is_ok()
    }
}
struct Shared {
    id: u64,
    settings: GenerationSettings,
    generator: std::sync::OnceLock<Result<Arc<TerrainGenerator>, String>>,
    initialization_waiters: Mutex<Vec<async_channel::Sender<Result<(), String>>>>,
    storage: Option<Arc<dyn ChunkStorage>>,
    cancelled: AtomicBool,
    view: Mutex<Option<ViewDemand>>,
    requests: Mutex<Vec<Request>>,
    snapshots: Mutex<FxHashMap<ChunkPos, ChunkSnapshot>>,
    entities: Mutex<FxHashMap<ChunkPos, Entity>>,
    flush: Mutex<Vec<async_channel::Sender<Result<(), String>>>>,
    paused: AtomicBool,
    loading: AtomicBool,
    errors: Mutex<Vec<String>>,
}

/// Immutable session handle. Procedural data is never replaced by player edits.
#[derive(Resource, Clone)]
pub struct GenerationSession(Arc<Shared>);
impl GenerationSession {
    pub fn new(seed: i64, dimension: Dimension, storage: Option<Arc<dyn ChunkStorage>>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(Arc::new(Shared {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            settings: GenerationSettings { seed, dimension },
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
    fn generator(&self) -> Arc<TerrainGenerator> {
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
        let r = i32::from(radius);
        let cells = (-r..=r)
            .flat_map(|x| (-r..=r).map(move |z| IVec2::new(x, z)))
            .filter(|p| p.length_squared() <= r * r)
            .map(|p| center + p)
            .collect();
        self.set_data_view(center, cells);
    }
    /// Exact data demand, including diagonals of the caller's rendering/simulation halo.
    pub fn set_data_view(&self, center: IVec2, mut cells: Vec<IVec2>) {
        cells.sort_unstable_by_key(|p| (p.x, p.y));
        cells.dedup();
        let mut admission: Vec<_> = (0..cells.len())
            .map(|i| u32::try_from(i).expect("Demand fits u32 indices"))
            .collect();
        admission.sort_unstable_by_key(|&i| {
            let p = cells[i as usize];
            (
                crate::coordinates::chunk_distance_squared(p, center),
                p.x,
                p.y,
            )
        });
        let radius = cells
            .iter()
            .map(|p| (*p - center).abs().max_element())
            .max()
            .unwrap_or(0)
            .min(255) as u8;
        *self.0.view.lock().unwrap() = Some(ViewDemand {
            center,
            radius,
            cells: cells.into(),
            admission: admission.into(),
        });
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
    /// Whether terrain is published here, without cloning its layers.
    pub fn has_snapshot(&self, position: IVec2) -> bool {
        self.0
            .snapshots
            .lock()
            .unwrap()
            .contains_key(&ChunkPos::new(position.x, position.y))
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
    index: usize,
    references: [u32; Stage::COUNT],
    chunk: Option<Chunk>,
    stage: Stage,
    revision: u64,
    saved_revision: Option<u64>,
    readers: u32,
    writing: bool,
    failed: bool,
    queued: bool,
    retirement_queued: bool,
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
    entries: FxHashMap<ChunkPos, Entry>,
    roots: FxHashSet<ChunkPos>,
    root_order: VecDeque<ChunkPos>,
    roots_to_scan: usize,
    view_cursor: usize,
    view: Option<ViewDemand>,
    changes: VecDeque<(ChunkPos, Stage, bool)>,
    positions: Vec<ChunkPos>,
    retiring: VecDeque<ChunkPos>,
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
            entries: FxHashMap::default(),
            roots: FxHashSet::default(),
            root_order: VecDeque::new(),
            roots_to_scan: 0,
            view_cursor: 0,
            view: None,
            changes: VecDeque::new(),
            positions: Vec::new(),
            retiring: VecDeque::new(),
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
            let center = self.view.as_ref().map_or(IVec2::ZERO, |view| view.center);
            let distance = crate::coordinates::chunk_distance_squared(
                IVec2::new(position.x, position.y),
                center,
            ) as u32;
            self.ready.push(Reverse((distance, position.x, position.y)));
        }
    }

    fn retire(&mut self, position: ChunkPos) {
        if let Some(entry) = self.entries.get_mut(&position)
            && entry.target() == Stage::None
            && !entry.retirement_queued
        {
            entry.retirement_queued = true;
            self.retiring.push_back(position);
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
    Initialized(Result<Arc<TerrainGenerator>, String>),
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
        if !app.is_plugin_added::<hollow_engine::tasks::VoxelTaskPlugin>() {
            app.add_plugins(hollow_engine::tasks::VoxelTaskPlugin);
        }
        app.init_resource::<GenerationGraph>()
            .add_message::<ChunkAvailable>()
            .add_observer(
                |event: On<Remove<ChunkRetention>>,
                 chunks: Query<&hollow_engine::chunk::VoxelChunk>,
                 mut graph: ResMut<GenerationGraph>| {
                    if let Ok(chunk) = chunks.get(event.entity) {
                        let position = ChunkPos::new(chunk.position.x, chunk.position.y);
                        for runtime in &mut graph.0 {
                            if runtime
                                .entries
                                .get(&position)
                                .is_some_and(|entry| entry.entity == event.entity)
                            {
                                runtime.retire(position);
                            }
                        }
                    }
                },
            )
            .add_systems(PostUpdate, collect.in_set(VoxelTasks::Collect))
            .add_systems(PostUpdate, update_demands.in_set(VoxelTasks::Demand))
            .add_systems(PostUpdate, schedule.in_set(VoxelTasks::Requests));
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
        let view = runtime.session.0.view.lock().unwrap().clone();
        let unchanged = match (&runtime.view, &view) {
            (Some(a), Some(b)) => Arc::ptr_eq(&a.cells, &b.cells),
            (None, None) => true,
            _ => false,
        };
        if !unchanged {
            runtime.view = view.clone();
            runtime.trim_pending = view.as_ref().map(|view| (view.center, view.radius));
            runtime.roots_to_scan = runtime.root_order.len();
            runtime.view_cursor = 0;
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
                let keep = view
                    .as_ref()
                    .is_some_and(|view| view.contains(IVec2::new(position.x, position.y)));
                if keep {
                    runtime.root_order.push_back(position);
                } else {
                    runtime.roots.remove(&position);
                    runtime.changes.push_back((position, Stage::Full, false));
                }
            } else if let Some(view) = &view
                && runtime.view_cursor < view.cells.len()
            {
                let cell = view.cells[view.admission[runtime.view_cursor] as usize];
                runtime.view_cursor += 1;
                let position = ChunkPos::new(cell.x, cell.y);
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
                let index = runtime.positions.len();
                runtime.positions.push(position);
                Entry {
                    entity,
                    index,
                    references: [0; Stage::COUNT],
                    chunk: None,
                    stage: Stage::None,
                    revision: 0,
                    saved_revision: None,
                    readers: 0,
                    writing: false,
                    failed: false,
                    queued: false,
                    retirement_queued: false,
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
            if !add && propagate {
                runtime.retire(position);
            }
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
    for (entity, task) in tasks
        .contiguous_iter_mut()
        .expect("Dense generation task query")
        .flat_map(|(entities, tasks)| entities.iter().copied().zip(tasks))
    {
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
        for (position, _) in task.writes.iter().chain(&task.reads) {
            runtime.retire(*position);
        }
        // Only completions and demand changes wake stage admission. A blocked
        // chunk waits for its neighbors instead of scanning the whole view again.
        // Reservations overlap: wake each dilated neighbor once rather than
        // hashing up to 162 overlapping positions per completed task.
        let mut wake: Vec<(i32, i32)> = task
            .writes
            .iter()
            .chain(task.reads.iter())
            .flat_map(|(position, _)| {
                (-1..=1)
                    .flat_map(move |dx| (-1..=1).map(move |dz| (position.x + dx, position.y + dz)))
            })
            .collect();
        wake.sort_unstable();
        wake.dedup();
        for (x, z) in wake {
            runtime.queue(ChunkPos::new(x, z));
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
                // Published terrain is also owned by the application session,
                // which may already have despawned it while generation drained.
                commands.entity(entry.entity).try_despawn();
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
                        let generator = TerrainGenerator::new(
                            Seed(settings.seed as u64),
                            settings.dimension.pumpkin(),
                        );
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
                remaining: runtime.positions.len(),
                chunks: Vec::new(),
                writes: Vec::new(),
                replies: std::mem::take(&mut *runtime.session.0.flush.lock().unwrap()),
            });
            while capture.remaining > 0 && Instant::now() < deadline.until {
                capture.remaining -= 1;
                let position = runtime.positions[capture.remaining];
                let entry = runtime.entries.get_mut(&position).unwrap();
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
        let mut evicted = Vec::new();
        let mut evicted_writes = Vec::new();
        // Reference changes must settle before a temporary zero can retire a dependency.
        let retire_count = if runtime.changes.is_empty() {
            runtime.retiring.len().min(32)
        } else {
            0
        };
        for _ in 0..retire_count {
            if Instant::now() >= deadline.until {
                break;
            }
            let position = runtime.retiring.pop_front().unwrap();
            let entry = runtime.entries.get_mut(&position).unwrap();
            entry.retirement_queued = false;
            if entry.target() != Stage::None
                || entry.writing
                || entry.readers != 0
                || retained.contains(entry.entity)
            {
                continue;
            }
            if entry.chunk.is_some()
                && runtime.session.0.storage.is_some()
                && entry.saved_revision != Some(entry.revision)
            {
                if runtime.active < budget.worker_count() {
                    entry.writing = true;
                    evicted_writes.push((position, entry.revision));
                    evicted.push((position, entry.chunk.take().unwrap()));
                } else {
                    runtime.retire(position);
                }
            } else {
                let entry = runtime.entries.remove(&position).unwrap();
                // Dense positions support constant-time removal and budgeted flush capture.
                runtime.positions.swap_remove(entry.index);
                if let Some(swapped) = runtime.positions.get(entry.index) {
                    runtime.entries.get_mut(swapped).unwrap().index = entry.index;
                }
                runtime.session.0.entities.lock().unwrap().remove(&position);
                runtime
                    .session
                    .0
                    .snapshots
                    .lock()
                    .unwrap()
                    .remove(&position);
                commands.entity(entry.entity).despawn();
                IoTaskPool::get()
                    .spawn(async move {
                        drop(entry);
                    })
                    .detach();
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
                                [target; 9],
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
            let mut targets = [Stage::Full; 9];
            for dx in -read_radius..=read_radius {
                for dz in -read_radius..=read_radius {
                    let p = position.add_raw(dx, dz);
                    let entry = runtime.entries.get_mut(&p).unwrap();
                    if dx.abs().max(dz.abs()) <= write_radius {
                        entry.writing = true;
                        writes.push((p, entry.revision));
                        targets[cache.chunks.len()] = entry.target();
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
                    targets,
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
    prepare: impl FnOnce(&TerrainGenerator) -> Result<(Cache, bool), String>,
    first: Option<Stage>,
    target: Stage,
    read_radius: i32,
    targets: [Stage; 9],
) -> Output {
    let mut stage = first.unwrap_or(Stage::Empty);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let generator = session.generator();
        let (mut cache, saved) = prepare(&generator)?;
        for chunk in &mut cache.chunks {
            if let Chunk::Proto(chunk) = chunk {
                for section in &mut chunk.block_sections {
                    section.resume();
                }
            }
        }
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
        // Only a column parked at its requested stage is compressed. Active
        // generation keeps direct dense reads/writes, including neighbor writes.
        for (chunk, target) in cache.chunks.iter_mut().zip(targets) {
            if let Chunk::Proto(chunk) = chunk
                && target != Stage::None
                && chunk.stage >= target
            {
                for section in &mut chunk.block_sections {
                    section.compact();
                }
            }
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

/// Inspected allocations only: running task captures and native structure caches remain unattributed.
#[cfg(feature = "dev")]
#[derive(Default, Debug)]
pub struct GenerationMemory {
    pub working_bytes: usize,
    pub shared_bytes: usize,
    pub metadata_lower_bound: usize,
    pub template_bytes_lower_bound: usize,
    pub templates: usize,
    pub in_flight: usize,
    pub density_bytes: [usize; 3],
    pub density_buffer_counts: [[usize; 8]; 2],
}
#[cfg(feature = "dev")]
pub fn memory_census(
    world: &mut World,
    census: &mut hollow_core::sections::AllocationCensus,
) -> GenerationMemory {
    use crate::generation::structure::template::global_cache;
    let mut memory = GenerationMemory {
        density_bytes: crate::generation::noise::router::density_volume::memory_bytes(),
        density_buffer_counts:
            crate::generation::noise::router::density_volume::memory_buffer_counts(),
        metadata_lower_bound: world
            .query::<&GenerationTask>()
            .iter(world)
            .map(|task| {
                (task.writes.capacity() + task.reads.capacity()) * size_of::<(ChunkPos, u64)>()
            })
            .sum(),
        ..Default::default()
    };
    let before = census.bytes;
    if let Some(graph) = world.get_resource::<GenerationGraph>() {
        for runtime in &graph.0 {
            memory.in_flight += runtime.active;
            memory.metadata_lower_bound += runtime.entries.capacity()
                * size_of::<(ChunkPos, Entry)>()
                + runtime.positions.capacity() * size_of::<ChunkPos>()
                + (runtime.retiring.capacity()
                    + runtime.root_order.capacity()
                    + runtime.roots.capacity())
                    * size_of::<ChunkPos>()
                + runtime.changes.capacity() * size_of::<(ChunkPos, Stage, bool)>()
                + runtime.ready.capacity() * size_of::<Reverse<(u32, i32, i32)>>()
                + runtime.requests.capacity() * size_of::<Request>()
                + runtime.session.0.entities.lock().unwrap().capacity()
                    * size_of::<(ChunkPos, Entity)>()
                + runtime.session.0.snapshots.lock().unwrap().capacity()
                    * size_of::<(ChunkPos, ChunkSnapshot)>();
            let captured = runtime
                .flush_capture
                .iter()
                .flat_map(|capture| capture.chunks.iter().map(|(_, chunk)| chunk));
            for chunk in runtime
                .entries
                .values()
                .filter_map(|entry| entry.chunk.as_ref())
                .chain(captured)
            {
                memory.working_bytes += census_chunk(chunk, census);
            }
            if let Some(capture) = &runtime.flush_capture {
                memory.metadata_lower_bound += capture.chunks.capacity()
                    * size_of::<(ChunkPos, Chunk)>()
                    + capture.writes.capacity() * size_of::<(ChunkPos, u64)>()
                    + capture.replies.capacity()
                        * size_of::<async_channel::Sender<Result<(), String>>>();
            }
            for snapshot in runtime.session.0.snapshots.lock().unwrap().values() {
                snapshot.states.census(census);
                snapshot.light.census(census);
                census.shared(&snapshot.biomes);
                census.shared(&snapshot.fluid_ticks);
            }
        }
    }
    memory.shared_bytes = census.bytes - before;
    let before = census.bytes;
    memory.templates = global_cache().census(census);
    memory.template_bytes_lower_bound = census.bytes - before;
    memory
}

#[cfg(feature = "dev")]
fn census_chunk(chunk: &Chunk, census: &mut hollow_core::sections::AllocationCensus) -> usize {
    use crate::generation::{proto_chunk::WorkingSection, structure::template::nbt_bytes};
    let mut working = 0;
    match chunk {
        Chunk::Proto(chunk) => {
            working += size_of::<ProtoChunk>()
                + chunk.block_sections.capacity() * size_of::<WorkingSection>()
                + size_of_val(chunk.flat_biome_map.as_ref())
                + chunk.fluid_ticks.capacity()
                    * size_of::<crate::tick::ScheduledTick<&'static pumpkin_data::fluid::Fluid>>();
            working += chunk.surface_estimates.capacity() * size_of::<(u64, i32)>()
                + (chunk.pending_block_entities.capacity()
                    + chunk.pending_structure_entities.capacity())
                    * size_of::<pumpkin_nbt::compound::NbtCompound>()
                + chunk
                    .pending_block_entities
                    .iter()
                    .chain(&chunk.pending_structure_entities)
                    .map(nbt_bytes)
                    .sum::<usize>();
            for section in &chunk.block_sections {
                match section {
                    WorkingSection::Dense(cells) => working += size_of_val(cells.as_ref()),
                    WorkingSection::Compact(cells) => cells.census(census),
                    WorkingSection::Uniform(_) => {}
                }
            }
            for channel in [&chunk.light.sky_light, &chunk.light.block_light] {
                working += size_of_val(channel.as_ref());
                for section in channel.iter() {
                    if let crate::chunk::format::LightContainer::Full(cells) = section {
                        working += cells.len();
                    }
                }
            }
        }
        Chunk::Complete(chunk) if census.shared(chunk) => {
            chunk.section.states.read().unwrap().census(census);
            chunk.light.lock().unwrap().census(census);
            census.shared(&chunk.section.biomes);
            census.shared(&chunk.fluid_ticks);
        }
        _ => {}
    }
    working
}
