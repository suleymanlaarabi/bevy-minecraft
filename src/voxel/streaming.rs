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

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;
    use crate::voxel::{SetVoxel, VoxelKind, VoxelPlugin};

    #[test]
    fn streaming_is_bounded_persistent_and_idle() {
        assert_eq!(StreamOffsets::new(8).len(), 197);
        let settings = VoxelSettings {
            view_distance: 0,
            spawn_budget_per_frame: 1,
            despawn_budget_per_frame: 1,
            ..default()
        };
        assert_eq!(
            settings.split_world_position(IVec3::new(-1, 3, -17)),
            (IVec2::new(-1, -2), IVec3::new(15, 3, 15))
        );
        let generated = generate_chunk(IVec2::new(2, -3), &settings);
        assert_eq!(
            generated.solid_positions(),
            generate_chunk(IVec2::new(2, -3), &settings).solid_positions()
        );

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            VoxelPlugin::new(settings.clone()),
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
        app.finish();
        let viewer = app
            .world_mut()
            .spawn((VoxelViewer, Transform::default()))
            .id();
        app.world_mut()
            .trigger(SetVoxel::new(IVec3::ZERO, VoxelKind::Air));
        let origin = wait_for_chunk(&mut app, IVec2::ZERO);
        assert!(
            app.world_mut()
                .get_mut::<ChunkVoxels>(origin)
                .unwrap()
                .set(IVec3::X, VoxelKind::Air)
        );
        app.update();
        app.world_mut()
            .entity_mut(viewer)
            .insert(Transform::from_xyz(
                settings.chunk_size as f32 * 3.0,
                0.0,
                0.0,
            ));
        wait_for_chunk(&mut app, IVec2::new(3, 0));
        assert!(!app.world().entities().contains(origin));
        assert!(
            !app.world()
                .resource::<ChunkIndex>()
                .contains_key(&IVec2::ZERO)
        );
        app.world_mut()
            .entity_mut(viewer)
            .insert(Transform::default());
        let reloaded = wait_for_chunk(&mut app, IVec2::ZERO);
        let voxels = app.world().get::<ChunkVoxels>(reloaded).unwrap();
        assert_eq!(voxels.get(IVec3::ZERO), Some(VoxelKind::Air));
        assert_eq!(voxels.get(IVec3::X), Some(VoxelKind::Air));
        for _ in 0..100 {
            app.update();
        }
        assert_eq!(app.world().resource::<ChunkIndex>().len(), 1);
    }

    fn wait_for_chunk(app: &mut App, position: IVec2) -> Entity {
        for _ in 0..2_000 {
            app.update();
            thread::yield_now();
            if let Some(&entity) = app.world().resource::<ChunkIndex>().get(&position)
                && app.world().get::<Collider>(entity).is_some()
            {
                return entity;
            }
        }
        panic!("chunk {position} did not finish loading");
    }
}
