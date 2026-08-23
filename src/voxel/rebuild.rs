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
    if let Some(entity) = index.get(&chunk)
        && let Ok(mut voxels) = chunks.get_mut(*entity)
    {
        voxels.set(local, event.kind);
        return;
    }
    let voxels = stored
        .entry(chunk)
        .or_insert_with(|| super::generation::generate_chunk(chunk, &settings));
    voxels.set(local, event.kind);
}

pub(crate) fn cleanup_removed_chunk(
    event: On<Remove, ChunkVoxels>,
    chunks: Query<(&super::VoxelChunk, &ChunkVoxels)>,
    mut index: ResMut<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
) {
    if let Ok((chunk, voxels)) = chunks.get(event.entity) {
        index.remove(&chunk.position);
        if voxels.modified {
            stored.insert(chunk.position, voxels.clone());
        }
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
