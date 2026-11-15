use super::texture_pack::TexturePack;
use bevy::prelude::*;
use bevy_voxel::render::assets::{MaterialTextures, RenderAssetStores, VoxelAssets};
use bevy_voxel::render::material::{ModelBlend, WaterMaterial};

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
    let textures = textures(&pack, &asset_server, &fog, &mut stores);
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
    mut stores: RenderAssetStores,
) {
    let (Some(texture_pack), Some(voxel_assets)) = (texture_pack.as_deref_mut(), voxel_assets)
    else {
        return;
    };
    let next_pack = TexturePack::load(settings.texture_pack);
    if texture_pack.texture_path == next_pack.texture_path {
        return;
    }

    let textures = textures(&next_pack, &asset_server, &fog, &mut stores);
    stores.apply(&voxel_assets, textures);
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

fn textures(
    pack: &TexturePack,
    asset_server: &AssetServer,
    fog: &bevy_voxel::render::fog::SkyPalette,
    stores: &mut RenderAssetStores,
) -> MaterialTextures {
    let blocks = pack.load_texture(asset_server, &mut stores.images);
    let (cube_faces, model_faces) = pack.face_buffers();
    MaterialTextures {
        blocks,
        faces: stores.buffers.add(cube_faces),
        model_faces: stores.buffers.add(model_faces),
        animations: stores.buffers.add(pack.animation_buffer()),
        special: pack.special_texture(asset_server),
        water: prepare_water(asset_server, pack.surface_water(), fog),
        opaque: opaque_blend(pack.vanilla_models()),
    }
}
