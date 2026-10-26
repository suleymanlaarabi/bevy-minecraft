use super::{
    ChunkVoxels, SetVoxel, VoxelChunk, VoxelSettings, VoxelViewer,
    generation::{ChunkLoad, WorldGenerator},
    lighting::LitEdits,
    render::build::{ChunkSections, NeedsChunkBuild},
};
use crate::game::GameState;
use bevy::{
    platform::collections::{HashMap, HashSet, hash_map::Entry},
    prelude::*,
    tasks::{IoTaskPool, Task, futures::check_ready},
};
use hollow_worldgen::coordinates::neighbor_slot;
use hollow_worldgen::coordinates::{ALL_SECTIONS, CHUNK_SIZE, chunk_at};
#[cfg(feature = "dev")]
use std::time::{Duration, Instant};
use std::{cmp::Reverse, collections::BinaryHeap};

/// Gameplay records being loaded and materialized around shared generated sections.
const MAX_PENDING_LOADS: usize = 16;

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ChunkIndex(HashMap<IVec2, Entity>);
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct StoredChunks(HashMap<IVec2, ChunkVoxels>);
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct PendingChunks(HashMap<IVec2, Entity>);
/// Edits aimed at chunks that are neither loaded nor stored, replayed when they spawn.
/// Generating a chunk just to edit it would block the main thread.
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct PendingEdits(HashMap<IVec2, Vec<SetVoxel>>);

#[derive(Component)]
pub(crate) struct ChunkGeneration {
    position: IVec2,
    task: Task<GeneratedChunk>,
}

/// A spawned chunk that is not meshed until its four neighbors have filled its halo,
/// so it is meshed once instead of once per arriving neighbor.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct AwaitingNeighbors;

struct GeneratedChunk {
    result: ChunkLoad,
    #[cfg(feature = "dev")]
    elapsed: Duration,
}

/// Chunks kept loaded around the viewer, nearest first.
///
/// Data extends one ring past the render radius so every rendered chunk can be
/// meshed once with all its neighbors present.
#[derive(Resource, Deref)]
pub(crate) struct StreamOffsets {
    /// Loading radius: the render radius plus one ring of data-only chunks.
    pub radius: u32,
    pub render_radius: u32,
    #[deref]
    offsets: Vec<IVec2>,
}

impl StreamOffsets {
    pub(crate) fn new(render_radius: u32) -> Self {
        let radius = render_radius as i32 + 1;
        let mut offsets: Vec<_> = (-radius..=radius)
            .flat_map(|x| (-radius..=radius).map(move |z| IVec2::new(x, z)))
            .filter(|offset| offset.length_squared() <= radius * radius)
            .collect();
        offsets.sort_unstable_by_key(|offset| offset.length_squared());
        Self {
            radius: radius as u32,
            render_radius,
            offsets,
        }
    }
}

/// Arrival notifications are reused by neighbor refresh and mesh readiness.
#[derive(Resource, Default)]
pub(crate) struct ChunkArrivals {
    pending: Vec<IVec2>,
    current: Vec<IVec2>,
}

pub(super) fn track_chunk_arrival(
    event: On<Add<VoxelChunk>>,
    chunks: Query<&VoxelChunk>,
    mut arrivals: ResMut<ChunkArrivals>,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        arrivals.pending.push(chunk.position);
    }
}

#[derive(Resource, Default)]
pub(crate) struct StreamState {
    pub center: Option<IVec2>,
    pub pending_despawns: Vec<Entity>,
    /// Positions in the loaded area with neither an entity nor a gameplay load in flight.
    missing: HashSet<IVec2>,
    /// Missing positions whose data is available, nearest first (may hold duplicates).
    ready: BinaryHeap<Reverse<(i32, i32, i32)>>,
}

impl StreamState {
    fn push_ready(&mut self, position: IVec2) {
        let center = self.center.unwrap_or_default();
        self.ready.push(Reverse((
            hollow_worldgen::coordinates::chunk_distance_squared(position, center),
            position.x,
            position.y,
        )));
    }

    fn retry_ready(&mut self, position: IVec2, offsets: &StreamOffsets) {
        self.retry_later(position, offsets);
        if self.missing.contains(&position) {
            self.push_ready(position);
        }
    }

    /// A load that found its base unavailable waits for Bevy's next completion message.
    fn retry_later(&mut self, position: IVec2, offsets: &StreamOffsets) {
        let radius_squared = (offsets.radius * offsets.radius) as i32;
        if self.center.is_some_and(|center| {
            hollow_worldgen::coordinates::chunk_distance_squared(position, center) <= radius_squared
        }) {
            self.missing.insert(position);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stream_chunks(
    mut commands: Commands,
    viewer: Single<&GlobalTransform, With<VoxelViewer>>,
    settings: Res<VoxelSettings>,
    generator: Res<WorldGenerator>,
    mut available: MessageReader<hollow_worldgen::runtime::ChunkAvailable>,
    budget: Res<hollow_worldgen::tasks::VoxelTaskBudget>,
    offsets: Res<StreamOffsets>,
    mut stored: ResMut<StoredChunks>,
    mut pending: ResMut<PendingChunks>,
    mut edits: ResMut<PendingEdits>,
    mut index: ResMut<ChunkIndex>,
    mut state: ResMut<StreamState>,
    sections: Query<&ChunkSections>,
) {
    let center = chunk_at(viewer.translation());
    let radius_squared = (offsets.radius * offsets.radius) as i32;
    if state.center != Some(center) {
        state.pending_despawns.clear();
        state.pending_despawns.extend(
            index
                .iter()
                .filter(|(position, _)| {
                    hollow_worldgen::coordinates::chunk_distance_squared(**position, center)
                        > radius_squared
                })
                .map(|(_, &entity)| entity),
        );
        pending.retain(|position, entity| {
            let keep = hollow_worldgen::coordinates::chunk_distance_squared(*position, center)
                <= radius_squared;
            if !keep {
                commands.entity(*entity).despawn();
            }
            keep
        });
        state.center = Some(center);
        generator.set_view(center, offsets.radius);
        // One pass per crossed border; later frames only react to completion events.
        state.missing.clear();
        state.ready.clear();
        for &offset in offsets.iter() {
            let position = center + offset;
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
        .filter(|message| message.session == generator.runtime.id())
    {
        let position = message.position;
        if state.missing.contains(&position) {
            state.push_ready(position);
        }
    }

    let mut despawned = 0;
    while despawned < settings.despawn_budget_per_frame {
        let Some(entity) = state.pending_despawns.pop() else {
            break;
        };
        despawned += sections
            .get(entity)
            .map_or(1, ChunkSections::mesh_count)
            .max(1);
        unload_chunk(&mut commands, entity);
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
        let Some(chunk_entity) = generator.runtime.entity(position) else {
            state.retry_ready(position, &offsets);
            break;
        };
        if let Some(voxels) = stored.remove(&position) {
            spawn_chunk(
                &mut commands,
                &mut index,
                &mut edits,
                position,
                chunk_entity,
                voxels,
                default(),
            );
        } else {
            let generator = generator.clone();
            let budget = budget.clone();
            let task = IoTaskPool::get().spawn(async move {
                #[cfg(feature = "dev")]
                let started = Instant::now();
                let result = generator.loaded_gameplay_chunk(position, budget).await;
                GeneratedChunk {
                    result,
                    #[cfg(feature = "dev")]
                    elapsed: started.elapsed(),
                }
            });
            let entity = commands
                .spawn((
                    ChunkGeneration { position, task },
                    DespawnOnExit(GameState::Game),
                ))
                .id();
            pending.insert(position, entity);
        }
        spawned += 1;
    }
}

pub(crate) fn poll_chunk_generations(
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    generator: Res<WorldGenerator>,
    mut generations: Query<(Entity, &mut ChunkGeneration)>,
    mut stored: ResMut<StoredChunks>,
    mut pending: ResMut<PendingChunks>,
    mut edits: ResMut<PendingEdits>,
    mut index: ResMut<ChunkIndex>,
    mut state: ResMut<StreamState>,
    offsets: Res<StreamOffsets>,
    #[cfg(feature = "dev")] mut diagnostics: Option<ResMut<super::diagnostics::VoxelDiagnostics>>,
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
        pending.remove(&position);
        commands.entity(task_entity).despawn();
        completed += 1;
        let (voxels, containers, revision) = match generated.result {
            ChunkLoad::Ready {
                voxels,
                containers,
                revision,
            } => (voxels, containers, revision),
            ChunkLoad::Retry => {
                state.retry_ready(position, &offsets);
                continue;
            }
            ChunkLoad::Missing => {
                state.retry_later(position, &offsets);
                continue;
            }
        };
        if generator.storage_revision(position) != Some(revision) {
            state.retry_ready(position, &offsets);
            continue;
        }
        #[cfg(feature = "dev")]
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.record_generation(generated.elapsed);
        }
        if index.contains_key(&position) {
            continue;
        }
        let Some(chunk_entity) = generator.runtime.entity(position) else {
            state.retry_ready(position, &offsets);
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
            containers,
        );
    }
}

fn spawn_chunk(
    commands: &mut Commands,
    index: &mut ChunkIndex,
    edits: &mut PendingEdits,
    position: IVec2,
    chunk_entity: Entity,
    mut voxels: ChunkVoxels,
    containers: crate::block_behavior::containers::PreparedColumn,
) {
    let Entry::Vacant(index_entry) = index.entry(position) else {
        return;
    };
    voxels.dirty_sections = ALL_SECTIONS;
    let lit_edits = voxels.modified;
    let origin = position * CHUNK_SIZE;
    let mut entity = commands.entity(chunk_entity);
    entity.insert((
        VoxelChunk { position },
        voxels,
        Transform::from_xyz(origin.x as f32, 0.0, origin.y as f32),
        Visibility::default(),
        AwaitingNeighbors,
        hollow_worldgen::runtime::ChunkRetention,
    ));
    if lit_edits {
        entity.insert(LitEdits);
    }
    index_entry.insert(chunk_entity);
    crate::block_behavior::containers::spawn_for_chunk(
        commands,
        chunk_entity,
        position,
        containers,
    );
    // Queued after the spawn: `set_voxel` then finds the chunk and applies them as
    // ordinary edits, neighbors and lighting included.
    for edit in edits.remove(&position).into_iter().flatten() {
        commands.trigger(edit);
    }
}

/// Starts the first mesh of rendered chunks once their eight neighbors are loaded
/// (or outside the loaded area), so each is meshed once with complete borders.
pub(crate) fn release_ready_chunks(
    mut commands: Commands,
    index: Res<ChunkIndex>,
    offsets: Res<StreamOffsets>,
    state: Res<StreamState>,
    waiting: Query<(Entity, &VoxelChunk), With<AwaitingNeighbors>>,
    arrivals: Option<Res<ChunkArrivals>>,
    mut last_view: Local<Option<(IVec2, u32)>>,
    mut candidates: Local<HashSet<Entity>>,
) {
    let Some(center) = state.center else {
        return;
    };
    let render_squared = (offsets.render_radius * offsets.render_radius) as i32;
    let loaded_squared = (offsets.radius * offsets.radius) as i32;
    candidates.clear();
    if *last_view != Some((center, offsets.render_radius)) || arrivals.is_none() {
        candidates.extend(waiting.iter().map(|(entity, _)| entity));
        *last_view = Some((center, offsets.render_radius));
    } else if let Some(arrivals) = arrivals {
        for &position in &arrivals.current {
            for offset in NEIGHBOR_OFFSETS.into_iter().chain([IVec2::ZERO]) {
                if let Some(&entity) = index.get(&(position + offset))
                    && waiting.contains(entity)
                {
                    candidates.insert(entity);
                }
            }
        }
    }
    for &entity in candidates.iter() {
        let Ok((_, chunk)) = waiting.get(entity) else {
            continue;
        };
        if hollow_worldgen::coordinates::chunk_distance_squared(chunk.position, center)
            > render_squared
        {
            continue;
        }
        let ready = NEIGHBOR_OFFSETS.iter().all(|&offset| {
            let neighbor = chunk.position + offset;
            index.contains_key(&neighbor)
                || hollow_worldgen::coordinates::chunk_distance_squared(neighbor, center)
                    > loaded_squared
        });
        if ready {
            commands
                .entity(entity)
                .remove::<AwaitingNeighbors>()
                .insert(NeedsChunkBuild);
        }
    }
}

const NEIGHBOR_OFFSETS: [IVec2; 8] = [
    IVec2::new(-1, -1),
    IVec2::new(0, -1),
    IVec2::new(1, -1),
    IVec2::new(-1, 0),
    IVec2::new(1, 0),
    IVec2::new(-1, 1),
    IVec2::new(0, 1),
    IVec2::new(1, 1),
];

/// Remeshes chunks that were last built without a neighbor that just arrived.
pub(crate) fn refresh_neighbors_of_spawned(
    index: Res<ChunkIndex>,
    mut chunks: Query<(&VoxelChunk, &mut ChunkVoxels, Option<&ChunkSections>)>,
    mut arrivals: ResMut<ChunkArrivals>,
) {
    let arrivals = &mut *arrivals;
    arrivals.current.clear();
    std::mem::swap(&mut arrivals.current, &mut arrivals.pending);
    for &position in &arrivals.current {
        for offset in NEIGHBOR_OFFSETS {
            let Some(&entity) = index.get(&(position + offset)) else {
                continue;
            };
            let Ok((_, mut voxels, Some(sections))) = chunks.get_mut(entity) else {
                continue;
            };
            if sections.missing_neighbors & (1 << neighbor_slot(-offset)) != 0 {
                voxels.mark_sections(ALL_SECTIONS);
            }
        }
    }
}

pub(crate) fn cleanup_removed_chunk(
    event: On<Remove<ChunkVoxels>>,
    chunks: Query<(&VoxelChunk, &ChunkVoxels)>,
    mut index: ResMut<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
) {
    if let Ok((chunk, voxels)) = chunks.get(event.entity) {
        index.remove(&chunk.position);
        if voxels.save_revision != voxels.saved_revision {
            stored.insert(chunk.position, voxels.clone());
        }
    }
}

pub(super) fn cleanup_voxel_world(
    mut commands: Commands,
    generator: Option<Res<WorldGenerator>>,
    chunks: Query<Entity, With<VoxelChunk>>,
    generations: Query<Entity, With<ChunkGeneration>>,
) {
    for entity in &chunks {
        unload_chunk(&mut commands, entity);
    }
    for entity in &generations {
        commands.entity(entity).despawn();
    }
    // Release view demand; active generation retains its reservations until return.
    if let Some(generator) = generator {
        generator.clear_view();
    }
    // Despawn observers write back into these resources, so reset them afterwards.
    commands.queue(|world: &mut World| {
        world.resource_mut::<ChunkIndex>().clear();
        world.resource_mut::<PendingChunks>().clear();
        world.resource_mut::<PendingEdits>().clear();
        world.resource_mut::<StoredChunks>().clear();
        world
            .resource_mut::<crate::block_behavior::containers::GameplayColumns>()
            .clear();
        // Dispose shared lighting buffers off the ECS thread.
        if let Some(state) = world.remove_resource::<super::lighting::LightingState>() {
            bevy::tasks::IoTaskPool::get()
                .spawn(async move {
                    drop(state);
                })
                .detach();
        }
        world.init_resource::<super::lighting::LightingState>();
        *world.resource_mut::<super::lighting::BlockLightEdits>() = default();
        *world.resource_mut::<StreamState>() = StreamState::default();
        *world.resource_mut::<ChunkArrivals>() = ChunkArrivals::default();
    });
}

/// Release presentation while the generation graph retains the procedural column.
fn unload_chunk(commands: &mut Commands, entity: Entity) {
    let mut entity = commands.entity(entity);
    entity.despawn_children();
    // Observers capture edits before identity and ownership relationships disappear.
    entity.remove::<ChunkVoxels>();
    entity.remove::<VoxelChunk>();
    entity.remove::<(
        hollow_worldgen::runtime::ChunkRetention,
        crate::block_behavior::containers::LoadedColumn,
        ChunkSections,
        super::render::build::ChunkBuild,
        NeedsChunkBuild,
        AwaitingNeighbors,
        LitEdits,
        super::edits::EditedChunk,
    )>();
}
