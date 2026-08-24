use super::{
    ChunkVoxels, VoxelChunk, VoxelSettings, VoxelViewer,
    generation::{WorldGenerator, generate_chunk},
    rebuild::VoxelAssets,
};
use crate::{game::GameState, spatial::GameLayer};
use avian3d::prelude::*;
use bevy::{
    platform::collections::{HashMap, hash_map::Entry},
    prelude::*,
};

const CHUNK_DIRECTIONS: [IVec2; 4] = [IVec2::NEG_X, IVec2::X, IVec2::NEG_Y, IVec2::Y];
type AddedChunks<'w, 's> =
    Query<'w, 's, (Entity, &'static VoxelChunk, &'static ChunkVoxels), Added<ChunkVoxels>>;
type MutableChunks<'w, 's> = Query<'w, 's, &'static mut ChunkVoxels>;

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ChunkIndex(HashMap<IVec2, Entity>);
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct StoredChunks(HashMap<IVec2, ChunkVoxels>);

#[derive(Resource, Deref)]
pub(crate) struct StreamOffsets {
    pub radius: u32,
    #[deref]
    offsets: Vec<IVec2>,
}

impl StreamOffsets {
    pub(crate) fn new(radius: u32) -> Self {
        let radius = radius as i32;
        let mut offsets: Vec<_> = (-radius..=radius)
            .flat_map(|x| (-radius..=radius).map(move |z| IVec2::new(x, z)))
            .filter(|offset| offset.length_squared() <= radius * radius)
            .collect();
        offsets.sort_unstable_by_key(|offset| offset.length_squared());
        Self {
            radius: radius as u32,
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
    assets: Res<VoxelAssets>,
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
        state.center = Some(center);
        state.cursor = 0;
    }

    for _ in 0..settings.despawn_budget_per_frame {
        let Some(entity) = state.pending_despawns.pop() else {
            break;
        };
        commands.entity(entity).despawn();
    }

    let mut spawned = 0;
    while spawned < settings.spawn_budget_per_frame && state.cursor < offsets.len() {
        let position = center + offsets[state.cursor];
        state.cursor += 1;
        let Entry::Vacant(index_entry) = index.entry(position) else {
            continue;
        };
        let mut voxels = stored
            .remove(&position)
            .unwrap_or_else(|| generate_chunk(position, &settings, &generator));
        voxels.changes = super::data::MESH_CHANGED | super::data::COLLIDER_CHANGED;
        let origin = position * settings.chunk_size;
        let entity = commands
            .spawn((
                VoxelChunk { position },
                voxels,
                MeshMaterial3d(assets.terrain.clone()),
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
                DespawnOnExit(GameState::Game),
            ))
            .id();
        index_entry.insert(entity);
        spawned += 1;
    }
}

pub(crate) fn sync_spawned_halos(
    index: Res<ChunkIndex>,
    stored: Res<StoredChunks>,
    mut chunks: ParamSet<(AddedChunks, MutableChunks)>,
) {
    let spawned: Vec<_> = chunks
        .p0()
        .iter()
        .map(|(entity, chunk, voxels)| (entity, chunk.position, voxels.clone()))
        .collect();

    for (entity, position, snapshot) in spawned {
        let neighbors = CHUNK_DIRECTIONS.map(|direction| {
            let neighbor_position = position + direction;
            if let Some(entity) = index.get(&neighbor_position) {
                chunks
                    .p1()
                    .get_mut(*entity)
                    .ok()
                    .map(|neighbor| (direction, Some(*entity), neighbor.clone()))
            } else {
                stored
                    .get(&neighbor_position)
                    .cloned()
                    .map(|neighbor| (direction, None, neighbor))
            }
        });

        if let Ok(mut voxels) = chunks.p1().get_mut(entity) {
            for (direction, _, neighbor) in neighbors.iter().flatten() {
                voxels.sync_halo(*direction, neighbor);
            }
        }
        if snapshot.modified {
            for (direction, neighbor, _) in neighbors.iter().flatten() {
                if let Some(entity) = neighbor
                    && let Ok(mut neighbor) = chunks.p1().get_mut(*entity)
                {
                    let changed = neighbor
                        .bypass_change_detection()
                        .sync_halo(-*direction, &snapshot);
                    if changed {
                        neighbor.set_changed();
                    }
                }
            }
        }
    }
}
