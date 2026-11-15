use crate::{
    fog::SkyPalette,
    material::{ModelBlend, ModelMaterial, VoxelCutoutMaterial, VoxelMaterial, WaterMaterial},
};
use bevy::render::storage::ShaderBuffer;
use bevy::{
    prelude::*,
    render::render_resource::{TextureDimension, TextureViewDimension},
};
/// A one-layer image is a valid texture array, but its default GPU view is D2.
/// Fix shared texture views after image or material events and before render extraction.
pub fn prepare_single_layer_arrays(
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
    // Inspect the current handles as well when a material changes: an importer
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
pub struct VoxelAssets {
    pub terrain: Handle<VoxelMaterial>,
    /// Alpha-tested greedy cubes (leaves), sharing the terrain's texture and faces.
    pub cutout: Handle<VoxelCutoutMaterial>,
    /// Model quads with fully opaque textures.
    pub models: Handle<ModelMaterial>,
    /// Alpha-tested model quads (plants, torches...).
    pub models_cutout: Handle<ModelMaterial>,
    pub translucent: Handle<ModelMaterial>,
}

#[derive(bevy::ecs::system::SystemParam)]
pub struct RenderAssetStores<'w> {
    pub images: ResMut<'w, Assets<Image>>,
    pub buffers: ResMut<'w, Assets<ShaderBuffer>>,
    pub terrain: ResMut<'w, Assets<VoxelMaterial>>,
    pub cutout: ResMut<'w, Assets<VoxelCutoutMaterial>>,
    pub models: ResMut<'w, Assets<ModelMaterial>>,
}

/// Importers provide numeric face tables and shared texture handles once.
pub struct MaterialTextures {
    pub blocks: Handle<Image>,
    pub faces: Handle<ShaderBuffer>,
    pub model_faces: Handle<ShaderBuffer>,
    pub animations: Handle<ShaderBuffer>,
    pub special: Option<Handle<Image>>,
    pub water: WaterMaterial,
    pub opaque: ModelBlend,
}
impl RenderAssetStores<'_> {
    pub fn prepare(&mut self, textures: MaterialTextures, fog: &SkyPalette) -> VoxelAssets {
        let MaterialTextures {
            blocks,
            faces,
            model_faces,
            animations,
            special,
            water,
            opaque,
        } = textures;
        let [models, models_cutout, translucent] =
            [opaque, ModelBlend::Cutout, ModelBlend::Translucent].map(|blend| {
                self.models.add(ModelMaterial::new(
                    blocks.clone(),
                    model_faces.clone(),
                    &water,
                    blend,
                    special.clone(),
                    animations.clone(),
                ))
            });
        VoxelAssets {
            models,
            models_cutout,
            translucent,
            cutout: self.cutout.add(VoxelCutoutMaterial {
                blocks: blocks.clone(),
                faces: faces.clone(),
                animations: animations.clone(),
                fog: fog.0.clone(),
            }),
            terrain: self.terrain.add(VoxelMaterial {
                blocks,
                faces,
                animations,
                fog: fog.0.clone(),
            }),
        }
    }
}

/// Sampler shared by block arrays, imported images and their generated mip chains.
pub fn block_sampler() -> bevy::image::ImageSampler {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    })
}

pub fn load_block_texture(server: &AssetServer, path: &str, layers: u32) -> Handle<Image> {
    use bevy::image::{ImageArrayLayout, ImageLoaderSettings};
    server
        .load_builder()
        .with_settings(move |settings: &mut ImageLoaderSettings| {
            settings.array_layout =
                (layers > 1).then_some(ImageArrayLayout::RowCount { rows: layers });
            settings.sampler = block_sampler();
        })
        .load(path.to_owned())
}
