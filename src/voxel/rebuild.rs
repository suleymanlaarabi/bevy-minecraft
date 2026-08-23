use super::{
    ChunkVoxels, SetVoxel, VoxelSettings,
    data::COLLIDER_CHANGED,
    generation::WorldGenerator,
    material::{VoxelMaterial, VoxelMaterialExtension, block_texture},
    meshing::{ChunkMeshes, build_chunk_mesh},
    streaming::{ChunkIndex, StoredChunks},
};
use avian3d::prelude::*;
use bevy::{
    light::NotShadowCaster,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
#[derive(Resource)]
pub(crate) struct VoxelAssets {
    pub(crate) terrain: Handle<VoxelMaterial>,
    water: Handle<StandardMaterial>,
}

pub(crate) fn prepare_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let blocks = block_texture(&asset_server);
    commands.insert_resource(VoxelAssets {
        terrain: voxel_materials.add(VoxelMaterial {
            base: StandardMaterial {
                alpha_mode: AlphaMode::Mask(0.5),
                perceptual_roughness: 1.0,
                ..default()
            },
            extension: VoxelMaterialExtension { blocks },
        }),
        water: materials.add(StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            perceptual_roughness: 0.15,
            ..default()
        }),
    });
}

enum ColliderUpdate {
    Keep,
    Replace(Collider),
    Remove,
}
struct BuildOutput {
    meshes: ChunkMeshes,
    collider: ColliderUpdate,
}
#[derive(Component)]
pub(crate) struct ChunkWater {
    entity: Entity,
    mesh: Handle<Mesh>,
}
#[derive(Component)]
pub(crate) struct ChunkBuild {
    task: Task<BuildOutput>,
    flags: u8,
}
pub(crate) fn set_voxel(
    event: On<SetVoxel>,
    settings: Res<VoxelSettings>,
    generator: Res<WorldGenerator>,
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
        .or_insert_with(|| super::generation::generate_chunk(chunk, &settings, &generator));
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
            let meshes = build_chunk_mesh(&snapshot);
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
            BuildOutput { meshes, collider }
        });
        commands.entity(entity).insert(ChunkBuild { task, flags });
    }
}

pub(crate) fn poll_builds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<VoxelAssets>,
    mut chunks: Query<(
        Entity,
        &mut ChunkBuild,
        Option<&Mesh3d>,
        Option<&ChunkWater>,
    )>,
) {
    for (entity, mut build, mesh_handle, water) in &mut chunks {
        let Some(output) = check_ready(&mut build.task) else {
            continue;
        };
        let BuildOutput {
            meshes: ChunkMeshes(terrain, next_water),
            collider,
        } = output;
        if let Some(handle) = mesh_handle
            && let Some(mut mesh) = meshes.get_mut(&handle.0)
        {
            *mesh = terrain;
        } else {
            commands.entity(entity).insert(Mesh3d(meshes.add(terrain)));
        }
        match (next_water, water) {
            (Some(mesh), Some(water)) => *meshes.get_mut(&water.mesh).unwrap() = mesh,
            (Some(mesh), None) => {
                let mesh = meshes.add(mesh);
                let child = commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(assets.water.clone()),
                        ChildOf(entity),
                        NotShadowCaster,
                    ))
                    .id();
                commands.entity(entity).insert(ChunkWater {
                    entity: child,
                    mesh,
                });
            }
            (None, Some(water)) => {
                commands.entity(water.entity).despawn();
                commands.entity(entity).remove::<ChunkWater>();
            }
            (None, None) => {}
        }
        match collider {
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
#[path = "../voxel_rebuild_tests.rs"]
mod tests;
