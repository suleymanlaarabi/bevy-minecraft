//! Spatial demand and persistent nearest-first loading state.
use crate::{
    chunk::{ChunkVoxels, VoxelChunk},
    edits::{LitEdits, PendingEdits, StoredChunks},
    session::{ActiveSession, SessionMember},
    tasks::VoxelTaskBudget,
    world::{ChunkIndex, WorldLayout},
};
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::{
    prelude::*,
    system::{StaticSystemParam, SystemParam, SystemParamItem},
};
use bevy_math::IVec2;
use bevy_platform::collections::HashSet;
use bevy_platform::collections::{HashMap, hash_map::Entry};
use bevy_tasks::{IoTaskPool, Task, futures::check_ready};
use hollow_core::VoxelCell;
use std::{cmp::Reverse, collections::BinaryHeap};
use std::{
    future::Future,
    time::{Duration, Instant},
};
/// A spawned chunk that is not meshed until its four neighbors have filled its halo,
/// so it is meshed once instead of once per arriving neighbor.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct AwaitingNeighbors;

/// Chunks kept loaded around the viewer, nearest first.
///
/// Data extends one ring past the render radius so every rendered chunk can be
/// meshed once with all its neighbors present.
#[derive(Resource, Deref)]
pub struct StreamOffsets {
    /// Axis-aligned bound of the expanded render set (used by visibility).
    pub radius: u32,
    pub render_radius: u32,
    #[deref]
    offsets: Vec<IVec2>,
}

impl Default for StreamOffsets {
    fn default() -> Self {
        Self::new(4)
    }
}
impl StreamOffsets {
    pub fn new(render_radius: u32) -> Self {
        // The one-cell halo is a square dilation of the discrete circle.
        // Direct enumeration avoids nine insertions/hashes for every interior cell.
        assert!(
            render_radius <= 128,
            "Render radius exceeds bounded streaming storage"
        );
        let radius = render_radius as i32;
        let mut offsets = Vec::new();
        for x in -radius - 1..=radius + 1 {
            for z in -radius - 1..=radius + 1 {
                let inner = IVec2::new((x.abs() - 1).max(0), (z.abs() - 1).max(0));
                if inner.length_squared() <= radius * radius {
                    offsets.push(IVec2::new(x, z));
                }
            }
        }
        offsets.sort_unstable_by_key(|offset| (offset.length_squared(), offset.x, offset.y));
        Self {
            radius: radius as u32 + 1,
            render_radius,
            offsets,
        }
    }
}

#[derive(Resource, Default)]
pub struct StreamState {
    pub center: Option<IVec2>,
    pub simulation_center: Option<IVec2>,
    pub simulation_radius: u32,
    // Two independently centered regions can be arbitrarily far apart; a sparse
    // set avoids a dense table covering the entire gap. Rebuilt only on demand changes.
    pub data: HashSet<IVec2>,
    pub pending_despawns: Vec<Entity>,
    /// Positions in the loaded area with neither an entity nor a gameplay load in flight.
    pub missing: HashSet<IVec2>,
    /// Missing positions whose data is available, nearest first (may hold duplicates).
    pub ready: BinaryHeap<Reverse<(i32, i32, i32)>>,
}

impl StreamState {
    pub fn push_ready(&mut self, position: IVec2) {
        let center = self.center.unwrap_or_default();
        self.ready.push(Reverse((
            hollow_core::coordinates::chunk_distance_squared(position, center),
            position.x,
            position.y,
        )));
    }

    pub fn retry_ready(&mut self, position: IVec2) {
        self.retry_later(position);
        if self.missing.contains(&position) {
            self.push_ready(position);
        }
    }

    /// A load that found its base unavailable waits for Bevy's next completion message.
    pub fn retry_later(&mut self, position: IVec2) {
        if self.data.contains(&position) {
            self.missing.insert(position);
        }
    }
}

const MAX_PENDING_LOADS: usize = 16;
#[derive(Resource, Default, Deref, DerefMut)]
pub struct PendingChunks(HashMap<IVec2, Entity>);
#[derive(Resource, Default)]
pub struct StreamDemand {
    pub center: Option<IVec2>,
    pub simulation_center: Option<IVec2>,
    pub simulation_radius: u32,
}
#[derive(Resource)]
pub struct StreamSettings {
    pub spawn_budget_per_frame: usize,
    pub despawn_budget_per_frame: usize,
}
impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            spawn_budget_per_frame: 8,
            despawn_budget_per_frame: 96,
        }
    }
}
#[derive(Message)]
pub struct TerrainAvailable {
    pub session: Entity,
    pub position: IVec2,
}
#[derive(Message)]
pub struct TerrainPublished {
    pub elapsed: Duration,
}
#[derive(Message)]
pub struct TerrainFailure {
    pub session: Entity,
    pub position: IVec2,
    pub reason: String,
}
pub enum ChunkLoad<T: VoxelCell, A> {
    Ready {
        voxels: ChunkVoxels<T>,
        attachment: A,
        revision: u64,
    },
    Retry,
    Missing,
    Failed(String),
}
/// Hooks execute per column, not per cell. Policies declare their real ECS accesses.
pub trait TerrainProvider: Resource + Clone {
    type Cell: VoxelCell;
    type Attachment: Default + Send + Sync + 'static;
    type Policy: SystemParam + 'static;
    fn set_view(&self, center: IVec2, positions: Vec<IVec2>);
    fn is_ready(&self, position: IVec2) -> bool;
    fn storage_revision(&self, position: IVec2) -> Option<u64>;
    fn entity(&self, commands: &mut Commands, position: IVec2) -> Option<Entity>;
    fn load(
        &self,
        position: IVec2,
        budget: VoxelTaskBudget,
    ) -> impl Future<Output = ChunkLoad<Self::Cell, Self::Attachment>> + Send + 'static;
    fn materialize(
        &self,
        commands: &mut Commands,
        entity: Entity,
        position: IVec2,
        attachment: Self::Attachment,
    );
    fn release(&self, commands: &mut Commands, entity: Entity);
    fn retirement_budget(policy: &SystemParamItem<Self::Policy>) -> (usize, usize);
    fn retained_bytes(policy: &SystemParamItem<Self::Policy>, entity: Entity) -> usize;
    fn retirement_weight(policy: &SystemParamItem<Self::Policy>, entity: Entity) -> usize;
}
#[derive(Component)]
pub struct ChunkGeneration<P: TerrainProvider> {
    session: Entity,
    position: IVec2,
    task: Task<GeneratedChunk<P>>,
}
struct GeneratedChunk<P: TerrainProvider> {
    result: ChunkLoad<P::Cell, P::Attachment>,
    elapsed: Duration,
}
#[allow(clippy::too_many_arguments)]
pub fn stream_chunks<P: TerrainProvider>(
    mut commands: Commands,
    demand: Res<StreamDemand>,
    session: Res<ActiveSession>,
    settings: Res<StreamSettings>,
    generator: Res<P>,
    mut available: MessageReader<TerrainAvailable>,
    budget: Res<VoxelTaskBudget>,
    offsets: Res<StreamOffsets>,
    mut stored: ResMut<StoredChunks<P::Cell>>,
    mut pending: ResMut<PendingChunks>,
    mut edits: ResMut<PendingEdits<P::Cell>>,
    mut index: ResMut<ChunkIndex>,
    mut state: ResMut<StreamState>,
    policy: StaticSystemParam<P::Policy>,
) {
    let Some(center) = demand.center else {
        return;
    };
    let simulation_center = demand.simulation_center.unwrap_or(center);
    let simulation_radius = demand.simulation_radius.min(128);
    if state.center != Some(center)
        || state.simulation_center != Some(simulation_center)
        || state.simulation_radius != simulation_radius
    {
        state.data.clear();
        state
            .data
            .extend(offsets.iter().map(|offset| center + offset));
        let radius = simulation_radius as i32;
        for x in -radius..=radius {
            for z in -radius..=radius {
                let offset = IVec2::new(x, z);
                if offset.length_squared() > radius * radius {
                    continue;
                }
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        state
                            .data
                            .insert(simulation_center + offset + IVec2::new(dx, dz));
                    }
                }
            }
        }
        state.simulation_center = Some(simulation_center);
        state.simulation_radius = simulation_radius;
        state.pending_despawns.clear();
        let state = &mut *state;
        state.pending_despawns.extend(
            index
                .iter()
                .filter(|(position, _)| !state.data.contains(*position))
                .map(|(_, &entity)| entity),
        );
        pending.retain(|position, entity| {
            let keep = state.data.contains(position);
            if !keep {
                commands.entity(*entity).despawn();
            }
            keep
        });
        state.center = Some(center);
        generator.set_view(center, state.data.iter().copied().collect());
        // One pass per crossed border; later frames only react to completion events.
        state.missing.clear();
        state.ready.clear();
        let positions: Vec<_> = state.data.iter().copied().collect();
        for position in positions {
            if index.contains_key(&position) || pending.contains_key(&position) {
                continue;
            }
            state.missing.insert(position);
            if stored.contains_key(&position) || generator.is_ready(position) {
                state.push_ready(position);
            }
        }
    }

    for message in available
        .read()
        .filter(|message| message.session == session.0)
    {
        let position = message.position;
        if state.missing.contains(&position) {
            state.push_ready(position);
        }
    }

    let (mut writeback_bytes, max_retired_bytes) = P::retirement_budget(&policy);
    let mut blocked_retirement = false;
    let mut despawned = 0;
    while despawned < settings.despawn_budget_per_frame {
        let Some(&entity) = state.pending_despawns.last() else {
            break;
        };
        let retained = P::retained_bytes(&policy, entity);
        if retained != 0 && writeback_bytes.saturating_add(retained) > max_retired_bytes {
            blocked_retirement = true;
            break;
        }
        writeback_bytes += retained;
        state.pending_despawns.pop();
        despawned += P::retirement_weight(&policy, entity).max(1);
        generator.release(&mut commands, entity);
    }

    if blocked_retirement || writeback_bytes >= max_retired_bytes {
        return;
    }
    let mut spawned = 0;
    while spawned < settings.spawn_budget_per_frame && pending.len() < MAX_PENDING_LOADS {
        let Some(Reverse((_, x, z))) = state.ready.pop() else {
            break;
        };
        let position = IVec2::new(x, z);
        if !state.missing.remove(&position) {
            continue;
        }
        if !stored.contains_key(&position) && generator.storage_revision(position).is_none() {
            state.retry_ready(position);
            break;
        }
        if stored.contains_key(&position) {
            let Some(chunk_entity) = generator.entity(&mut commands, position) else {
                state.retry_ready(position);
                break;
            };
            let voxels = stored.remove(&position).expect("Retained column");
            spawn_chunk(
                &mut commands,
                &mut index,
                &mut edits,
                position,
                chunk_entity,
                voxels,
                Default::default(),
                &*generator,
                session.0,
            );
        } else {
            let generator = generator.clone();
            let budget = budget.clone();
            let task = IoTaskPool::get().spawn(async move {
                let started = Instant::now();
                let result = generator.load(position, budget).await;
                GeneratedChunk::<P> {
                    result,
                    elapsed: started.elapsed(),
                }
            });
            let entity = commands
                .spawn((
                    ChunkGeneration::<P> {
                        session: session.0,
                        position,
                        task,
                    },
                    SessionMember(session.0),
                ))
                .id();
            pending.insert(position, entity);
        }
        spawned += 1;
    }
}

pub fn poll_chunk_generations<P: TerrainProvider>(
    mut commands: Commands,
    settings: Res<StreamSettings>,
    generator: Res<P>,
    mut generations: Query<(Entity, &mut ChunkGeneration<P>)>,
    mut stored: ResMut<StoredChunks<P::Cell>>,
    mut pending: ResMut<PendingChunks>,
    mut edits: ResMut<PendingEdits<P::Cell>>,
    mut index: ResMut<ChunkIndex>,
    mut state: ResMut<StreamState>,
    session: Res<ActiveSession>,
    mut published: MessageWriter<TerrainPublished>,
    layout: Res<WorldLayout>,
    mut failures: MessageWriter<TerrainFailure>,
) {
    let mut completed = 0;
    for (task_entity, mut generation) in &mut generations {
        if completed >= settings.spawn_budget_per_frame {
            break;
        }
        let Some(generated) = check_ready(&mut generation.task) else {
            continue;
        };
        let position = generation.position;
        if generation.session != session.0 {
            commands.entity(task_entity).despawn();
            continue;
        }
        pending.remove(&position);
        commands.entity(task_entity).despawn();
        completed += 1;
        let (voxels, attachment, revision) = match generated.result {
            ChunkLoad::Ready {
                voxels,
                attachment,
                revision,
            } => (voxels, attachment, revision),
            ChunkLoad::Retry => {
                state.retry_ready(position);
                continue;
            }
            ChunkLoad::Failed(reason) => {
                failures.write(TerrainFailure {
                    session: session.0,
                    position,
                    reason,
                });
                continue;
            }
            ChunkLoad::Missing => {
                state.retry_later(position);
                continue;
            }
        };
        if generator.storage_revision(position) != Some(revision) {
            state.retry_ready(position);
            continue;
        }
        if !state.data.contains(&position) {
            continue;
        }
        if voxels.states.section_count() != layout.section_count()
            || voxels.light.section_count() != layout.section_count()
        {
            failures.write(TerrainFailure {
                session: session.0,
                position,
                reason: "Provider column does not match world layout".into(),
            });
            continue;
        }
        published.write(TerrainPublished {
            elapsed: generated.elapsed,
        });
        if index.contains_key(&position) {
            continue;
        }
        let Some(chunk_entity) = generator.entity(&mut commands, position) else {
            state.retry_ready(position);
            continue;
        };
        let voxels = stored.remove(&position).unwrap_or(voxels);
        spawn_chunk(
            &mut commands,
            &mut index,
            &mut edits,
            position,
            chunk_entity,
            voxels,
            attachment,
            &*generator,
            session.0,
        );
    }
}

fn spawn_chunk<P: TerrainProvider>(
    commands: &mut Commands,
    index: &mut ChunkIndex,
    edits: &mut PendingEdits<P::Cell>,
    position: IVec2,
    chunk_entity: Entity,
    mut voxels: ChunkVoxels<P::Cell>,
    attachment: P::Attachment,
    provider: &P,
    session: Entity,
) {
    let Entry::Vacant(index_entry) = index.entry(position) else {
        return;
    };
    voxels.dirty_sections = hollow_core::coordinates::section_mask(voxels.states.section_count());
    let lit_edits = voxels.modified;
    commands
        .entity(chunk_entity)
        .insert((VoxelChunk { position }, voxels, SessionMember(session)));
    if lit_edits {
        commands.entity(chunk_entity).insert(LitEdits);
    }
    index_entry.insert(chunk_entity);
    provider.materialize(commands, chunk_entity, position, attachment);
    // Queued after the spawn: `set_voxel` then finds the chunk and applies them as
    // ordinary edits, neighbors and lighting included.
    for edit in edits.remove(&position).into_iter().flatten() {
        commands.trigger(edit);
    }
}

pub fn cleanup_removed_chunk<T: VoxelCell>(
    event: On<Remove<ChunkVoxels<T>>>,
    chunks: Query<(&VoxelChunk, &ChunkVoxels<T>)>,
    mut index: ResMut<ChunkIndex>,
    mut stored: ResMut<StoredChunks<T>>,
) {
    if let Ok((chunk, voxels)) = chunks.get(event.entity) {
        index.remove(&chunk.position);
        if voxels.save_revision != voxels.saved_revision {
            stored.insert(chunk.position, voxels.clone());
        }
    }
}

#[derive(bevy_ecs::schedule::SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoxelPipeline;
#[derive(bevy_ecs::schedule::SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoxelStreamSystems {
    Demand,
    Publish,
}
pub struct StreamingPlugin<P: TerrainProvider>(std::marker::PhantomData<P>);
impl<P: TerrainProvider> Default for StreamingPlugin<P> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}
impl<P: TerrainProvider> bevy_app::Plugin for StreamingPlugin<P> {
    fn build(&self, app: &mut bevy_app::App) {
        app.init_resource::<PendingChunks>()
            .add_observer(reset_stream)
            .init_resource::<StreamState>()
            .init_resource::<StreamDemand>()
            .init_resource::<StreamSettings>()
            .init_resource::<StreamOffsets>()
            .add_message::<TerrainAvailable>()
            .add_message::<TerrainPublished>()
            .add_message::<TerrainFailure>()
            .add_systems(
                bevy_app::Update,
                stream_chunks::<P>
                    .in_set(VoxelStreamSystems::Demand)
                    .run_if(
                        bevy_ecs::schedule::common_conditions::resource_exists::<P>.and_then(
                            bevy_ecs::schedule::common_conditions::resource_exists::<ActiveSession>,
                        ),
                    ),
            )
            .add_systems(
                bevy_app::PostUpdate,
                poll_chunk_generations::<P>
                    .in_set(VoxelStreamSystems::Publish)
                    .in_set(VoxelPipeline)
                    .after(crate::tasks::VoxelTasks::Publish)
                    .before(crate::tasks::VoxelTasks::Requests)
                    .run_if(
                        bevy_ecs::schedule::common_conditions::resource_exists::<P>.and_then(
                            bevy_ecs::schedule::common_conditions::resource_exists::<ActiveSession>,
                        ),
                    ),
            );
    }
}

fn reset_stream(
    _: On<crate::session::SessionClosed>,
    mut pending: ResMut<PendingChunks>,
    mut stream: ResMut<StreamState>,
    mut demand: ResMut<StreamDemand>,
) {
    pending.clear();
    *stream = Default::default();
    *demand = Default::default();
}
