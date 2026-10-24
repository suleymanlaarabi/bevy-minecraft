use super::{
    ChunkVoxels, VoxelChunk, VoxelSettings, VoxelViewer,
    generation::WorldGenerator,
    rebuild::{ChunkSections, NeedsChunkBuild, neighbor_slot},
};
use crate::{game::GameState, spatial::GameLayer};
use avian3d::prelude::*;
use bevy::{
    platform::collections::{HashMap, hash_map::Entry},
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
#[cfg(feature = "dev")]
use std::time::{Duration, Instant};

/// Copies of finished Pumpkin chunks in flight; Pumpkin itself does the generation.
const MAX_PENDING_COPIES: usize = 16;

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ChunkIndex(HashMap<IVec2, Entity>);
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct StoredChunks(HashMap<IVec2, ChunkVoxels>);
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct PendingChunks(HashMap<IVec2, Entity>);

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

#[derive(Resource, Default)]
pub(crate) struct StreamState {
    pub center: Option<IVec2>,
    pub cursor: usize,
    pub pending_despawns: Vec<Entity>,
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
    mut index: ResMut<ChunkIndex>,
    mut state: ResMut<StreamState>,
) {
    let center = settings.chunk_at(viewer.translation());
    if state.center != Some(center) {
        let radius_squared = (offsets.radius * offsets.radius) as i32;
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
        state.cursor = 0;
        generator.set_view(center, offsets.radius);
    }

    // Newly finished chunks may fill positions the cursor already skipped.
    let radius_squared = (offsets.radius * offsets.radius) as i32;
    if generator
        .finished_chunks()
        .fold(false, |any, position| {
            any | (position.distance_squared(center) <= radius_squared)
        })
    {
        state.cursor = 0;
    }

    for _ in 0..settings.despawn_budget_per_frame {
        let Some(entity) = state.pending_despawns.pop() else {
            break;
        };
        commands.entity(entity).despawn();
    }

    let mut spawned = 0;
    while spawned < settings.spawn_budget_per_frame
        && pending.len() < MAX_PENDING_COPIES
        && state.cursor < offsets.len()
    {
        let position = center + offsets[state.cursor];
        if index.contains_key(&position) || pending.contains_key(&position) {
            state.cursor += 1;
            continue;
        }
        if let Some(voxels) = stored.remove(&position) {
            spawn_chunk(&mut commands, &settings, &mut index, position, voxels);
        } else if generator.is_loaded(position) {
            let settings = settings.clone();
            let generator = generator.clone();
            let task = AsyncComputeTaskPool::get().spawn(async move {
                #[cfg(feature = "dev")]
                let started = Instant::now();
                let voxels = generator.loaded_chunk(position, &settings)?;
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
        } else {
            // Still generating: its completion resets the cursor.
            state.cursor += 1;
            continue;
        }
        state.cursor += 1;
        spawned += 1;
    }
}

pub(crate) fn poll_chunk_generations(
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    mut generations: Query<(Entity, &mut ChunkGeneration)>,
    mut stored: ResMut<StoredChunks>,
    mut pending: ResMut<PendingChunks>,
    mut index: ResMut<ChunkIndex>,
    mut state: ResMut<StreamState>,
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
            state.cursor = 0;
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
        spawn_chunk(&mut commands, &settings, &mut index, position, voxels);
        completed += 1;
    }
}

fn spawn_chunk(
    commands: &mut Commands,
    settings: &VoxelSettings,
    index: &mut ChunkIndex,
    position: IVec2,
    mut voxels: ChunkVoxels,
) {
    let Entry::Vacant(index_entry) = index.entry(position) else {
        return;
    };
    voxels.dirty_sections = voxels.all_sections();
    voxels.collider_dirty = true;
    let origin = position * settings.chunk_size;
    let entity = commands
        .spawn((
            VoxelChunk { position },
            voxels,
            Transform::from_xyz(origin.x as f32, 0.0, origin.y as f32),
            RigidBody::Static,
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
        ))
        .id();
    index_entry.insert(entity);
}

/// Starts the first mesh of rendered chunks once their eight neighbors are loaded
/// (or outside the loaded area), so each is meshed once with complete borders.
pub(crate) fn release_ready_chunks(
    mut commands: Commands,
    index: Res<ChunkIndex>,
    offsets: Res<StreamOffsets>,
    state: Res<StreamState>,
    waiting: Query<(Entity, &VoxelChunk), With<AwaitingNeighbors>>,
) {
    let Some(center) = state.center else {
        return;
    };
    let render_squared = (offsets.render_radius * offsets.render_radius) as i32;
    let loaded_squared = (offsets.radius * offsets.radius) as i32;
    for (entity, chunk) in &waiting {
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
    mut spawned: Local<Vec<IVec2>>,
) {
    // One query: an `Added<ChunkVoxels>` filter would conflict with `&mut ChunkVoxels`.
    spawned.clear();
    spawned.extend(
        chunks
            .iter_mut()
            .filter(|(_, voxels, _)| voxels.is_added())
            .map(|(chunk, _, _)| chunk.position),
    );
    for &position in spawned.iter() {
        for offset in NEIGHBOR_OFFSETS {
            let Some(&entity) = index.get(&(position + offset)) else {
                continue;
            };
            let Ok((_, mut voxels, Some(sections))) = chunks.get_mut(entity) else {
                continue;
            };
            if sections.missing_neighbors & (1 << neighbor_slot(-offset)) != 0 {
                let all = voxels.all_sections();
                voxels.mark_sections(all);
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
