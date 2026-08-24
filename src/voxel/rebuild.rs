use super::{
    ChunkVoxels, SetVoxel, VoxelSettings,
    data::COLLIDER_CHANGED,
    generation::WorldGenerator,
    material::{
        TexturePack, VoxelMaterial, VoxelMaterialExtension, WaterMaterial, WaterMaterialExtension,
        block_texture,
    },
    meshing::{ChunkMeshes, build_chunk_mesh},
    streaming::{ChunkIndex, StoredChunks},
};
use avian3d::prelude::*;
use bevy::{
    light::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
#[derive(Resource)]
pub(crate) struct VoxelAssets {
    pub(crate) terrain: Handle<VoxelMaterial>,
    water: Handle<WaterMaterial>,
}

pub(crate) fn prepare_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Option<Res<crate::settings::GraphicsSettings>>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
) {
    let pack = TexturePack::load(
        settings
            .as_deref()
            .map_or(crate::voxel::TexturePackId::default(), |settings| {
                settings.texture_pack
            }),
    );
    let blocks = block_texture(&asset_server, &pack.texture_path);
    commands.insert_resource(pack);
    commands.insert_resource(VoxelAssets {
        terrain: voxel_materials.add(VoxelMaterial {
            base: StandardMaterial {
                perceptual_roughness: 1.0,
                ..default()
            },
            extension: VoxelMaterialExtension { blocks },
        }),
        water: water_materials.add(WaterMaterial {
            base: StandardMaterial {
                base_color: Color::linear_rgba(0.035, 0.30, 0.38, 0.56),
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                perceptual_roughness: 0.08,
                reflectance: 0.75,
                ..default()
            },
            extension: WaterMaterialExtension::default(),
        }),
    });
}

pub(crate) fn apply_texture_pack(
    _insert: On<Insert, crate::settings::GraphicsSettings>,
    settings: Res<crate::settings::GraphicsSettings>,
    asset_server: Res<AssetServer>,
    mut texture_pack: Option<ResMut<TexturePack>>,
    voxel_assets: Option<ResMut<VoxelAssets>>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
) {
    let (Some(texture_pack), Some(voxel_assets)) = (texture_pack.as_deref_mut(), voxel_assets)
    else {
        return;
    };
    if texture_pack.texture_path == TexturePack::load(settings.texture_pack).texture_path {
        return;
    }

    let next_pack = TexturePack::load(settings.texture_pack);
    if let Some(mut material) = voxel_materials.get_mut(&voxel_assets.terrain) {
        material.extension.blocks = block_texture(&asset_server, &next_pack.texture_path);
    }
    *texture_pack = next_pack;
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
    let changed = if let Some(entity) = index.get(&chunk) {
        let Ok(mut voxels) = chunks.get_mut(*entity) else {
            return;
        };
        let changed = voxels.bypass_change_detection().set(local, event.kind);
        if changed {
            voxels.set_changed();
        }
        changed
    } else {
        stored
            .entry(chunk)
            .or_insert_with(|| super::generation::generate_chunk(chunk, &settings, &generator))
            .set(local, event.kind)
    };
    if !changed {
        return;
    }

    for (neighbor_offset, neighbor_local) in border_halos(local, settings.chunk_size) {
        let position = chunk + neighbor_offset;
        if let Some(entity) = index.get(&position)
            && let Ok(mut voxels) = chunks.get_mut(*entity)
        {
            let changed = voxels
                .bypass_change_detection()
                .set_halo(neighbor_local, event.kind);
            if changed {
                voxels.set_changed();
            }
        } else if let Some(voxels) = stored.get_mut(&position) {
            voxels.set_halo(neighbor_local, event.kind);
        }
    }
}

fn border_halos(local: IVec3, size: i32) -> impl Iterator<Item = (IVec2, IVec3)> {
    [
        (local.x == 0).then_some((IVec2::NEG_X, IVec3::new(size, local.y, local.z))),
        (local.x == size - 1).then_some((IVec2::X, IVec3::new(-1, local.y, local.z))),
        (local.z == 0).then_some((IVec2::NEG_Y, IVec3::new(local.x, local.y, size))),
        (local.z == size - 1).then_some((IVec2::Y, IVec3::new(local.x, local.y, -1))),
    ]
    .into_iter()
    .flatten()
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
    texture_pack: Res<TexturePack>,
    mut chunks: Query<(Entity, &mut ChunkVoxels, Option<&ChunkBuild>), Changed<ChunkVoxels>>,
) {
    let pool = AsyncComputeTaskPool::get();
    for (entity, mut voxels, build) in &mut chunks {
        let changes = core::mem::take(&mut voxels.bypass_change_detection().changes);
        if changes == 0 {
            continue;
        }
        let snapshot = voxels.bypass_change_detection().clone();
        let texture_pack = texture_pack.clone();
        let flags = changes | build.map_or(0, |build| build.flags);
        let task = pool.spawn(async move {
            let meshes = build_chunk_mesh(&snapshot, &texture_pack);
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
    mut chunks: Query<(Entity, &mut ChunkBuild, Option<&ChunkWater>)>,
) {
    for (entity, mut build, water) in &mut chunks {
        let Some(output) = check_ready(&mut build.task) else {
            continue;
        };
        let BuildOutput {
            meshes: ChunkMeshes(terrain, next_water),
            collider,
        } = output;
        let terrain = meshes.add(terrain);
        commands.entity(entity).insert(Mesh3d(terrain));
        match (next_water, water) {
            (Some(mesh), Some(water)) => {
                commands
                    .entity(water.entity)
                    .insert(Mesh3d(meshes.add(mesh)));
            }
            (Some(mesh), None) => {
                let mesh = meshes.add(mesh);
                let child = commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(assets.water.clone()),
                        ChildOf(entity),
                        NotShadowCaster,
                        NotShadowReceiver,
                    ))
                    .id();
                commands.entity(entity).insert(ChunkWater { entity: child });
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
