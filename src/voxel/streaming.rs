use super::{
    ChunkVoxels, VoxelChunk, VoxelSettings, VoxelViewer, generation::generate_chunk,
    rebuild::VoxelAssets,
};
use avian3d::prelude::*;
use bevy::{
    platform::collections::{HashMap, hash_map::Entry},
    prelude::*,
};
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ChunkIndex(HashMap<IVec2, Entity>);
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct StoredChunks(HashMap<IVec2, ChunkVoxels>);

#[derive(Resource, Deref)]
pub(crate) struct StreamOffsets(Vec<IVec2>);

impl StreamOffsets {
    pub(crate) fn new(radius: u32) -> Self {
        let radius = radius as i32;
        let mut offsets: Vec<_> = (-radius..=radius)
            .flat_map(|x| (-radius..=radius).map(move |z| IVec2::new(x, z)))
            .filter(|offset| offset.length_squared() <= radius * radius)
            .collect();
        offsets.sort_unstable_by_key(|offset| offset.length_squared());
        Self(offsets)
    }
}

#[derive(Default)]
pub(crate) struct StreamState {
    center: Option<IVec2>,
    cursor: usize,
    pending_despawns: Vec<Entity>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stream_chunks(
    mut commands: Commands,
    viewer: Single<&GlobalTransform, With<VoxelViewer>>,
    settings: Res<VoxelSettings>,
    offsets: Res<StreamOffsets>,
    mut stored: ResMut<StoredChunks>,
    assets: Res<VoxelAssets>,
    mut index: ResMut<ChunkIndex>,
    mut state: Local<StreamState>,
) {
    let center = settings.chunk_at(viewer.translation());
    if state.center != Some(center) {
        let radius_squared = (settings.view_distance * settings.view_distance) as i32;
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
            .unwrap_or_else(|| generate_chunk(position, &settings));
        voxels.changes = super::data::MESH_CHANGED | super::data::COLLIDER_CHANGED;
        let origin = position * settings.chunk_size;
        let entity = commands
            .spawn((
                VoxelChunk { position },
                voxels,
                MeshMaterial3d(assets.0.clone()),
                Transform::from_xyz(origin.x as f32, 0.0, origin.y as f32),
                RigidBody::Static,
                Friction::new(0.8),
            ))
            .id();
        index_entry.insert(entity);
        spawned += 1;
    }
}
