use bevy::{
    image::{
        ImageAddressMode, ImageArrayLayout, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};

pub(crate) const TEXTURE_LAYERS: u32 = 9;
pub(crate) const BLOCK_TEXTURE: &str = "voxel/replete/blocks_array.png";

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct VoxelMaterialExtension {
    #[texture(100, dimension = "2d_array")]
    #[sampler(101)]
    pub(crate) blocks: Handle<Image>,
}

pub(crate) type VoxelMaterial = ExtendedMaterial<StandardMaterial, VoxelMaterialExtension>;

pub(crate) fn block_texture(asset_server: &AssetServer) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.array_layout = Some(ImageArrayLayout::RowCount {
                rows: TEXTURE_LAYERS,
            });
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Nearest,
                min_filter: ImageFilterMode::Nearest,
                mipmap_filter: ImageFilterMode::Nearest,
                ..default()
            });
        })
        .load(BLOCK_TEXTURE)
}

impl MaterialExtension for VoxelMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/voxel_material.wgsl".into()
    }
}
