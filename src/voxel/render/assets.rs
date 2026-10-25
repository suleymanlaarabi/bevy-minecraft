use super::{
    material::{ModelBlend, ModelMaterial, VoxelCutoutMaterial, VoxelMaterial, WaterMaterial},
    texture_pack::{TexturePack, block_texture},
};
use bevy::{prelude::*, render::storage::ShaderBuffer};

#[derive(Resource)]
pub(crate) struct VoxelAssets {
    pub(crate) terrain: Handle<VoxelMaterial>,
    /// Alpha-tested greedy cubes (leaves), sharing the terrain's texture and faces.
    pub(crate) cutout: Handle<VoxelCutoutMaterial>,
    /// Model quads with fully opaque textures.
    pub(crate) models: Handle<ModelMaterial>,
    /// Alpha-tested model quads (plants, torches...).
    pub(crate) models_cutout: Handle<ModelMaterial>,
    pub(crate) translucent: Handle<ModelMaterial>,
}

pub(crate) fn prepare_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Option<Res<crate::settings::GraphicsSettings>>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
    mut cutout_materials: ResMut<Assets<VoxelCutoutMaterial>>,
    mut model_materials: ResMut<Assets<ModelMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let pack = TexturePack::load(
        settings
            .as_deref()
            .map_or(crate::voxel::TexturePackId::default(), |settings| {
                settings.texture_pack
            }),
    );
    let blocks = block_texture(&asset_server, &pack.texture_path, pack.layers);
    let faces = buffers.add(pack.cube_face_buffer());
    let model_faces = buffers.add(pack.model_face_buffer());
    let vanilla_models = pack.vanilla_models();
    commands.insert_resource(pack);
    let water = WaterMaterial::load(&asset_server);
    let mut model = |blend| {
        model_materials.add(ModelMaterial::new(
            blocks.clone(),
            model_faces.clone(),
            &water,
            blend,
        ))
    };
    commands.insert_resource(VoxelAssets {
        models: model(opaque_blend(vanilla_models)),
        models_cutout: model(ModelBlend::Cutout),
        translucent: model(ModelBlend::Translucent),
        cutout: cutout_materials.add(VoxelCutoutMaterial {
            blocks: blocks.clone(),
            faces: faces.clone(),
        }),
        terrain: voxel_materials.add(VoxelMaterial { blocks, faces }),
    });
}

pub(crate) fn apply_texture_pack(
    _insert: On<Insert<crate::settings::GraphicsSettings>>,
    settings: Res<crate::settings::GraphicsSettings>,
    asset_server: Res<AssetServer>,
    mut texture_pack: Option<ResMut<TexturePack>>,
    voxel_assets: Option<Res<VoxelAssets>>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
    mut cutout_materials: ResMut<Assets<VoxelCutoutMaterial>>,
    mut model_materials: ResMut<Assets<ModelMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let (Some(texture_pack), Some(voxel_assets)) = (texture_pack.as_deref_mut(), voxel_assets)
    else {
        return;
    };
    if texture_pack.texture_path == TexturePack::load(settings.texture_pack).texture_path {
        return;
    }

    let next_pack = TexturePack::load(settings.texture_pack);
    let blocks = block_texture(&asset_server, &next_pack.texture_path, next_pack.layers);
    // Meshes only store cube appearances: the face table carries the pack's layers and tints.
    let cube_faces = buffers.add(next_pack.cube_face_buffer());
    if let Some(mut material) = voxel_materials.get_mut(&voxel_assets.terrain) {
        material.blocks = blocks.clone();
        material.faces = cube_faces.clone();
    }
    if let Some(mut material) = cutout_materials.get_mut(&voxel_assets.cutout) {
        material.blocks = blocks.clone();
        material.faces = cube_faces.clone();
    }
    let faces = buffers.add(next_pack.model_face_buffer());
    for (_, material) in model_materials.iter_mut() {
        material.blocks = blocks.clone();
        material.faces = faces.clone();
    }
    if let Some(mut material) = model_materials.get_mut(&voxel_assets.models) {
        material.blend = opaque_blend(next_pack.vanilla_models());
    }
    *texture_pack = next_pack;
}

/// Blend of the quads meshed as opaque. Their split relies on vanilla texture
/// opacity: another pack may put holes in those layers, so they get alpha-tested.
fn opaque_blend(vanilla_models: bool) -> ModelBlend {
    if vanilla_models {
        ModelBlend::Opaque
    } else {
        ModelBlend::Cutout
    }
}
