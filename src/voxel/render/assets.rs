use super::{
    material::{ModelBlend, ModelMaterial, VoxelCutoutMaterial, VoxelMaterial, WaterMaterial},
    texture_pack::{TexturePack, block_texture},
};
use bevy::{
    prelude::*,
    render::{
        render_resource::{TextureDimension, TextureViewDimension},
        storage::ShaderBuffer,
    },
};

/// A one-layer image is a valid texture array, but its default GPU view is D2.
/// Fix shared texture views after image or material events and before render extraction.
pub(super) fn prepare_single_layer_arrays(
    voxel_assets: Option<Res<VoxelAssets>>,
    materials: Res<Assets<ModelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let material = voxel_assets
        .as_deref()
        .and_then(|assets| materials.get(&assets.models));
    let arrays = material
        .map(|material| {
            [
                Some(material.blocks.id()),
                material.special.as_ref().map(Handle::id),
                material.still.as_ref().map(Handle::id),
            ]
        })
        .unwrap_or([None; 3]);
    // Inspect the current handles as well when a material changes: a pack switch
    // can reuse an image whose Added event has already been consumed.
    for id in arrays.into_iter().flatten() {
        let needs_array_view = images.get(id).is_some_and(|image| {
            image.texture_descriptor.dimension == TextureDimension::D2
                && image.texture_descriptor.size.depth_or_array_layers == 1
                && image
                    .texture_view_descriptor
                    .as_ref()
                    .and_then(|view| view.dimension)
                    != Some(TextureViewDimension::D2Array)
        });
        if needs_array_view && let Some(mut image) = images.get_mut(id) {
            image
                .texture_view_descriptor
                .get_or_insert_default()
                .dimension = Some(TextureViewDimension::D2Array);
        }
    }
}

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
    let animations = buffers.add(pack.animation_buffer());
    let special = pack.special_texture(&asset_server);
    let water = WaterMaterial::load(&asset_server, pack.surface_water());
    commands.insert_resource(pack);
    let mut model = |blend| {
        model_materials.add(ModelMaterial::new(
            blocks.clone(),
            model_faces.clone(),
            &water,
            blend,
            special.clone(),
            animations.clone(),
        ))
    };
    commands.insert_resource(VoxelAssets {
        models: model(opaque_blend(vanilla_models)),
        models_cutout: model(ModelBlend::Cutout),
        translucent: model(ModelBlend::Translucent),
        cutout: cutout_materials.add(VoxelCutoutMaterial {
            blocks: blocks.clone(),
            faces: faces.clone(),
            animations: animations.clone(),
        }),
        terrain: voxel_materials.add(VoxelMaterial {
            blocks,
            faces,
            animations,
        }),
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
    let animations = buffers.add(next_pack.animation_buffer());
    let special = next_pack.special_texture(&asset_server);
    let water = WaterMaterial::load(&asset_server, next_pack.surface_water());
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
