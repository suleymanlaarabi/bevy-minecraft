use super::textures::MinecraftTextures;
use bevy::prelude::*;
use bevy_voxel::render::assets::{MaterialTextures, RenderAssetStores};
use bevy_voxel::render::material::{ModelBlend, WaterMaterial};

pub(crate) fn prepare_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fog: Res<bevy_voxel::render::fog::SkyPalette>,
    mut stores: RenderAssetStores,
) {
    let block_textures = MinecraftTextures::load();
    let materials = textures(&block_textures, &asset_server, &fog, &mut stores);
    commands.insert_resource(block_textures);
    commands.insert_resource(stores.prepare(materials, &fog));
}

fn prepare_water(
    asset_server: &AssetServer,
    surface: bool,
    fog: &bevy_voxel::render::fog::SkyPalette,
) -> WaterMaterial {
    let path = "voxel/packs/vanilla/water_still.png";
    let texture = (!surface && crate::asset_directory().join(path).is_file())
        .then(|| super::textures::block_texture(asset_server, path, 32));
    WaterMaterial::new(texture, 32, 10.0, surface, fog)
}

fn textures(
    textures: &MinecraftTextures,
    asset_server: &AssetServer,
    fog: &bevy_voxel::render::fog::SkyPalette,
    stores: &mut RenderAssetStores,
) -> MaterialTextures {
    let blocks = textures.load_texture(asset_server, &mut stores.images);
    let (cube_faces, model_faces) = textures.face_buffers();
    MaterialTextures {
        blocks,
        faces: stores.buffers.add(cube_faces),
        model_faces: stores.buffers.add(model_faces),
        animations: stores.buffers.add(textures.animation_buffer()),
        special: textures.special_texture(asset_server),
        water: prepare_water(asset_server, textures.surface_water(), fog),
        opaque: ModelBlend::Opaque,
    }
}
