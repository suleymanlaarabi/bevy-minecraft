use crate::material::{ModelMaterial, VoxelCutoutMaterial, VoxelMaterial};
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
