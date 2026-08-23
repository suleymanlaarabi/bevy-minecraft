use bevy::{
    image::{
        ImageAddressMode, ImageArrayLayout, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
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

#[derive(Clone, Copy, Debug, Reflect, ShaderType)]
pub(crate) struct WaterSettings {
    /// Two normalized XZ wave directions packed as XY and ZW.
    directions: Vec4,
    /// Spatial frequencies packed as XY and animation speeds packed as ZW.
    frequency_speed: Vec4,
    /// Wave slopes packed as XY. ZW are reserved for future tuning.
    strength: Vec4,
}

impl Default for WaterSettings {
    fn default() -> Self {
        let first_direction = Vec2::new(0.8, 0.6).normalize();
        let second_direction = Vec2::new(-0.45, 0.89).normalize();
        Self {
            directions: Vec4::new(
                first_direction.x,
                first_direction.y,
                second_direction.x,
                second_direction.y,
            ),
            frequency_speed: Vec4::new(0.32, 0.57, 0.75, -0.48),
            strength: Vec4::new(0.10, 0.055, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub(crate) struct WaterMaterialExtension {
    #[uniform(100)]
    settings: WaterSettings,
}

pub(crate) type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterMaterialExtension>;

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

impl MaterialExtension for WaterMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/water_material.wgsl".into()
    }
}
