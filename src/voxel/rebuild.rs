use super::{
    ChunkVoxels, SetVoxel, VoxelSettings,
    data::COLLIDER_CHANGED,
    meshing::build_chunk_mesh,
    streaming::{ChunkIndex, StoredChunks},
};
use avian3d::prelude::*;
use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
#[derive(Resource)]
pub(crate) struct VoxelAssets(pub Handle<StandardMaterial>);

pub(crate) fn prepare_assets(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(VoxelAssets(materials.add(StandardMaterial {
        perceptual_roughness: 1.0,
        ..default()
    })));
}

enum ColliderUpdate {
    Keep,
    Replace(Collider),
    Remove,
}
struct BuildOutput {
    mesh: Mesh,
    collider: ColliderUpdate,
}
#[derive(Component)]
pub(crate) struct ChunkBuild {
    task: Task<BuildOutput>,
    flags: u8,
}
pub(crate) fn set_voxel(
    event: On<SetVoxel>,
    settings: Res<VoxelSettings>,
    index: Res<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
    mut chunks: Query<&mut ChunkVoxels>,
) {
    let (chunk, local) = settings.split_world_position(event.world_position);
    if let Some(entity) = index.0.get(&chunk)
        && let Ok(mut voxels) = chunks.get_mut(*entity)
    {
        voxels.set(local, event.kind);
        return;
    }
    let voxels = stored
        .0
        .entry(chunk)
        .or_insert_with(|| super::generation::generate_chunk(chunk, &settings));
    voxels.set(local, event.kind);
}

pub(crate) fn persist_removed_chunk(
    event: On<Remove, ChunkVoxels>,
    chunks: Query<(&super::VoxelChunk, &ChunkVoxels)>,
    mut stored: ResMut<StoredChunks>,
) {
    if let Ok((chunk, voxels)) = chunks.get(event.entity)
        && voxels.modified
    {
        stored.0.insert(chunk.position, voxels.clone());
    }
}

pub(crate) fn start_changed_builds(
    mut commands: Commands,
    mut chunks: Query<(Entity, &mut ChunkVoxels, Option<&ChunkBuild>), Changed<ChunkVoxels>>,
) {
    let pool = AsyncComputeTaskPool::get();
    for (entity, mut voxels, build) in &mut chunks {
        let changes = core::mem::take(&mut voxels.bypass_change_detection().changes);
        if changes == 0 {
            continue;
        }
        let snapshot = voxels.bypass_change_detection().clone();
        let flags = changes | build.map_or(0, |build| build.flags);
        let task = pool.spawn(async move {
            let mesh = build_chunk_mesh(&snapshot);
            let collider = if flags & COLLIDER_CHANGED == 0 {
                ColliderUpdate::Keep
            } else {
                let solids = snapshot.solid_positions();
                if solids.is_empty() {
                    ColliderUpdate::Remove
                } else {
                    ColliderUpdate::Replace(Collider::voxels(Vec3::ONE, &solids))
                }
            };
            BuildOutput { mesh, collider }
        });
        commands.entity(entity).insert(ChunkBuild { task, flags });
    }
}

pub(crate) fn poll_builds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunks: Query<(Entity, &mut ChunkBuild, Option<&Mesh3d>)>,
) {
    for (entity, mut build, mesh_handle) in &mut chunks {
        let Some(output) = check_ready(&mut build.task) else {
            continue;
        };
        if let Some(handle) = mesh_handle
            && let Some(mut mesh) = meshes.get_mut(&handle.0)
        {
            *mesh = output.mesh;
        } else {
            commands
                .entity(entity)
                .insert(Mesh3d(meshes.add(output.mesh)));
        }
        match output.collider {
            ColliderUpdate::Keep => {}
            ColliderUpdate::Replace(collider) => {
                commands.entity(entity).insert(collider);
            }
            ColliderUpdate::Remove => {
                commands.entity(entity).remove::<Collider>();
            }
        }
        commands.entity(entity).remove::<ChunkBuild>();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::data::MESH_CHANGED;
    use super::*;
    use bevy::time::TimeUpdateStrategy;

    #[test]
    fn snapshots_are_shared_and_changes_are_minimal() {
        let mut voxels =
            ChunkVoxels::generated(4, 4, vec![super::super::VoxelKind::Air; 64], vec![0; 16]);
        assert_eq!(
            core::mem::take(&mut voxels.changes),
            MESH_CHANGED | COLLIDER_CHANGED
        );
        assert!(voxels.set(IVec3::ZERO, super::super::VoxelKind::Stone));
        let snapshot = voxels.clone();
        assert!(voxels.shares_cells_with(&snapshot));
        assert_eq!(
            core::mem::take(&mut voxels.changes),
            MESH_CHANGED | COLLIDER_CHANGED
        );
        assert!(voxels.set(IVec3::ZERO, super::super::VoxelKind::Dirt));
        assert_eq!(core::mem::take(&mut voxels.changes), MESH_CHANGED);
        assert!(!voxels.shares_cells_with(&snapshot));
        assert!(voxels.set(IVec3::ZERO, super::super::VoxelKind::Air));
        assert!(!voxels.shares_cells_with(&snapshot));
        assert_eq!(
            core::mem::take(&mut voxels.changes),
            MESH_CHANGED | COLLIDER_CHANGED
        );
    }

    #[test]
    fn dynamic_body_lands_on_voxel_collider() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::default()))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                1.0 / 60.0,
            )));
        app.finish();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::voxels(Vec3::ONE, &[IVec3::ZERO]),
        ));
        let body = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::cuboid(0.5, 0.5, 0.5),
                Position(Vec3::new(0.5, 4.0, 0.5)),
            ))
            .id();
        for _ in 0..240 {
            app.update();
        }
        let y = app.world().get::<Position>(body).unwrap().y;
        assert!((1.20..1.30).contains(&y), "body stopped at y={y}");
    }
}
