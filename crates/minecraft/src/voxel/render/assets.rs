use super::texture_pack::TexturePack;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy_voxel::render::assets::{MaterialTextures, RenderAssetStores, VoxelAssets};
use bevy_voxel::render::material::{
    ModelBlend, ModelMaterial, VoxelCutoutMaterial, VoxelMaterial, WaterMaterial,
};

pub(crate) fn prepare_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Option<Res<crate::settings::GraphicsSettings>>,
    fog: Res<bevy_voxel::render::fog::SkyPalette>,
    mut stores: RenderAssetStores,
) {
    let pack = TexturePack::load(
        settings
            .as_deref()
            .map_or(crate::voxel::TexturePackId::default(), |settings| {
                settings.texture_pack
            }),
    );
    let blocks = pack.load_texture(&asset_server, &mut stores.images);
    let textures = MaterialTextures {
        blocks,
        faces: stores.buffers.add(pack.cube_face_buffer()),
        model_faces: stores.buffers.add(pack.model_face_buffer()),
        animations: stores.buffers.add(pack.animation_buffer()),
        special: pack.special_texture(&asset_server),
        water: prepare_water(&asset_server, pack.surface_water(), &fog),
        opaque: opaque_blend(pack.vanilla_models()),
    };
    commands.insert_resource(pack);
    commands.insert_resource(stores.prepare(textures, &fog));
}

pub(crate) fn apply_texture_pack(
    _insert: On<Insert<crate::settings::GraphicsSettings>>,
    settings: Res<crate::settings::GraphicsSettings>,
    fog: Res<bevy_voxel::render::fog::SkyPalette>,
    asset_server: Res<AssetServer>,
    mut texture_pack: Option<ResMut<TexturePack>>,
    voxel_assets: Option<Res<VoxelAssets>>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
    mut cutout_materials: ResMut<Assets<VoxelCutoutMaterial>>,
    mut model_materials: ResMut<Assets<ModelMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (Some(texture_pack), Some(voxel_assets)) = (texture_pack.as_deref_mut(), voxel_assets)
    else {
        return;
    };
    if texture_pack.texture_path == TexturePack::load(settings.texture_pack).texture_path {
        return;
    }

    let next_pack = TexturePack::load(settings.texture_pack);
    let blocks = next_pack.load_texture(&asset_server, &mut images);
    // Meshes only store cube appearances: the face table carries the pack's layers and tints.
    let cube_faces = buffers.add(next_pack.cube_face_buffer());
    let animations = buffers.add(next_pack.animation_buffer());
    let special = next_pack.special_texture(&asset_server);
    let water = prepare_water(&asset_server, next_pack.surface_water(), &fog);
    if let Some(mut material) = voxel_materials.get_mut(&voxel_assets.terrain) {
        material.blocks = blocks.clone();
        material.faces = cube_faces.clone();
        material.animations = animations.clone();
    }
    if let Some(mut material) = cutout_materials.get_mut(&voxel_assets.cutout) {
        material.blocks = blocks.clone();
        material.faces = cube_faces.clone();
        material.animations = animations.clone();
    }
    let faces = buffers.add(next_pack.model_face_buffer());
    for (_, material) in model_materials.iter_mut() {
        material.blocks = blocks.clone();
        material.faces = faces.clone();
        material.animations = animations.clone();
        material.special = special.clone();
        material.set_water(&water);
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

fn prepare_water(
    asset_server: &AssetServer,
    surface: bool,
    fog: &bevy_voxel::render::fog::SkyPalette,
) -> WaterMaterial {
    let path = "voxel/packs/vanilla/water_still.png";
    let texture = (!surface && crate::asset_directory().join(path).is_file())
        .then(|| super::texture_pack::block_texture(asset_server, path, 32));
    WaterMaterial::new(texture, 32, 10.0, surface, fog)
}
