use super::{
    ChunkVoxels, SetVoxel, VoxelChunk, VoxelSettings, VoxelViewer,
    coordinates::neighbor_slot,
    coordinates::{ALL_SECTIONS, CHUNK_SIZE, chunk_at},
    generation::WorldGenerator,
    lighting::LitEdits,
    render::build::{ChunkSections, NeedsChunkBuild},
};
use crate::{game::GameState, spatial::GameLayer};
use avian3d::prelude::*;
use bevy::{
    platform::collections::{HashMap, HashSet, hash_map::Entry},
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
#[cfg(feature = "dev")]
use std::time::{Duration, Instant};
use std::{cmp::Reverse, collections::BinaryHeap};

/// Copies of finished Pumpkin chunks in flight; Pumpkin itself does the generation.
const MAX_PENDING_COPIES: usize = 16;

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
    task: Task<Option<GeneratedChunk>>,
}

/// A spawned chunk that is not meshed until its four neighbors have filled its halo,
/// so it is meshed once instead of once per arriving neighbor.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct AwaitingNeighbors;

struct GeneratedChunk {
    voxels: ChunkVoxels,
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
    mut colliders: ResMut<super::collision::ColliderRequests>,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        arrivals.pending.push(chunk.position);
        colliders.0.insert(event.entity);
    }
}

#[derive(Resource, Default)]
pub(crate) struct StreamState {
    pub center: Option<IVec2>,
    pub pending_despawns: Vec<Entity>,
    /// Positions in the loaded area with neither an entity nor a copy in flight.
    missing: HashSet<IVec2>,
    /// Missing positions whose data is available, nearest first (may hold duplicates).
    ready: BinaryHeap<Reverse<(i32, i32, i32)>>,
}

impl StreamState {
    fn push_ready(&mut self, position: IVec2) {
        let center = self.center.unwrap_or_default();
        self.ready.push(Reverse((
            position.distance_squared(center),
            position.x,
            position.y,
        )));
    }

    /// A copy that found its chunk unloaded waits for Pumpkin's next completion event.
    fn retry_later(&mut self, position: IVec2, offsets: &StreamOffsets) {
        let radius_squared = (offsets.radius * offsets.radius) as i32;
        if self
            .center
            .is_some_and(|center| position.distance_squared(center) <= radius_squared)
        {
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
                .filter(|(position, _)| position.distance_squared(center) > radius_squared)
                .map(|(_, &entity)| entity),
        );
        pending.retain(|position, entity| {
            let keep = position.distance_squared(center) <= radius_squared;
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

    for position in generator.finished_chunks() {
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
        commands.entity(entity).despawn();
    }

    let mut spawned = 0;
    while spawned < settings.spawn_budget_per_frame && pending.len() < MAX_PENDING_COPIES {
        let Some(Reverse((_, x, z))) = state.ready.pop() else {
            break;
        };
        let position = IVec2::new(x, z);
        if !state.missing.remove(&position) {
            continue;
        }
        if let Some(voxels) = stored.remove(&position) {
            spawn_chunk(&mut commands, &mut index, &mut edits, position, voxels);
        } else {
            let generator = generator.clone();
            // Decoding is CPU work; the snapshot read uses a WAL reader that never waits on saves.
            let task = AsyncComputeTaskPool::get().spawn(async move {
                #[cfg(feature = "dev")]
                let started = Instant::now();
                let voxels = generator.loaded_chunk(position)?;
                Some(GeneratedChunk {
                    voxels,
                    #[cfg(feature = "dev")]
                    elapsed: started.elapsed(),
                })
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
        let Some(generated) = generated else {
            // Unloaded by Pumpkin before the copy: retry once it is back.
            state.retry_later(position, &offsets);
            continue;
        };
        #[cfg(feature = "dev")]
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.record_generation(generated.elapsed);
        }
        if index.contains_key(&position) {
            continue;
        }
        let voxels = stored.remove(&position).unwrap_or(generated.voxels);
        spawn_chunk(&mut commands, &mut index, &mut edits, position, voxels);
        completed += 1;
    }
}

fn spawn_chunk(
    commands: &mut Commands,
    index: &mut ChunkIndex,
    edits: &mut PendingEdits,
    position: IVec2,
    mut voxels: ChunkVoxels,
) {
    let Entry::Vacant(index_entry) = index.entry(position) else {
        return;
    };
    voxels.dirty_sections = ALL_SECTIONS;
    voxels.collider_dirty = true;
    let lit_edits = !voxels.native_edits.is_empty();
    let origin = position * CHUNK_SIZE;
    let mut entity = commands.spawn((
        VoxelChunk { position },
        voxels,
        Transform::from_xyz(origin.x as f32, 0.0, origin.y as f32),
        CollisionLayers::new(
            GameLayer::World,
            [
                GameLayer::Default,
                GameLayer::Player,
                GameLayer::DroppedItem,
            ],
        ),
        Friction::new(0.8),
        Visibility::default(),
        AwaitingNeighbors,
        DespawnOnExit(GameState::Game),
    ));
    if lit_edits {
        entity.insert(LitEdits);
    }
    index_entry.insert(entity.id());
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
        if chunk.position.distance_squared(center) > render_squared {
            continue;
        }
        let ready = NEIGHBOR_OFFSETS.iter().all(|&offset| {
            let neighbor = chunk.position + offset;
            index.contains_key(&neighbor) || neighbor.distance_squared(center) > loaded_squared
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_wait_for_all_loaded_neighbors_before_meshing() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<ChunkIndex>();
        world.insert_resource(StreamOffsets::new(2));
        world.insert_resource(StreamState {
            center: Some(IVec2::ZERO),
            ..default()
        });
        let spawn = |world: &mut World, position: IVec2| {
            let entity = world
                .spawn((VoxelChunk { position }, AwaitingNeighbors))
                .id();
            world.resource_mut::<ChunkIndex>().insert(position, entity);
            entity
        };
        let center = spawn(&mut world, IVec2::ZERO);
        for offset in &NEIGHBOR_OFFSETS[..7] {
            spawn(&mut world, *offset);
        }
        world.run_system_once(release_ready_chunks).unwrap();
        assert!(world.get::<AwaitingNeighbors>(center).is_some());
        spawn(&mut world, NEIGHBOR_OFFSETS[7]);
        world.run_system_once(release_ready_chunks).unwrap();
        assert!(world.get::<AwaitingNeighbors>(center).is_none());
        assert!(world.get::<NeedsChunkBuild>(center).is_some());
    }

    #[test]
    fn stream_offsets_are_unique_nearest_first_and_inside_radius() {
        let offsets = StreamOffsets::new(1);
        assert_eq!(offsets.radius, 2);
        assert_eq!(offsets.len(), 13);
        assert_eq!(offsets[0], IVec2::ZERO);
        assert!(
            offsets
                .windows(2)
                .all(|pair| pair[0].length_squared() <= pair[1].length_squared())
        );
        assert!(offsets.iter().all(|offset| offset.length_squared() <= 4));
        for (index, offset) in offsets.iter().enumerate() {
            assert!(!offsets[..index].contains(offset));
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
        commands.entity(entity).despawn();
    }
    for entity in &generations {
        commands.entity(entity).despawn();
    }
    // Let Pumpkin unload the area; the next session sets its own view.
    if let Some(generator) = generator {
        generator.clear_view();
    }
    // Despawn observers write back into these resources, so reset them afterwards.
    commands.queue(|world: &mut World| {
        world.resource_mut::<ChunkIndex>().clear();
        world.resource_mut::<PendingChunks>().clear();
        world.resource_mut::<PendingEdits>().clear();
        world.resource_mut::<StoredChunks>().clear();
        // Drop the context on its own worker: Pumpkin shutdown must not stall ECS.
        if let Some(state) = world.remove_resource::<super::lighting::LightingState>() {
            bevy::tasks::IoTaskPool::get()
                .spawn(async move {
                    drop(state);
                })
                .detach();
        }
        world.init_resource::<super::lighting::LightingState>();
        *world.resource_mut::<StreamState>() = StreamState::default();
        *world.resource_mut::<ChunkArrivals>() = ChunkArrivals::default();
        *world.resource_mut::<super::collision::ColliderRequests>() = default();
    });
}
