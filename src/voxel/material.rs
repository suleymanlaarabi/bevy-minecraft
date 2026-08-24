#![allow(clippy::needless_arbitrary_self_type)]

use bevy::{
    image::{
        ImageAddressMode, ImageArrayLayout, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    pbr::{ExtendedMaterial, Material, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};
use serde::Deserialize;

use super::data::VoxelKind;

pub(crate) const TEXTURE_LAYERS: u32 = 9;

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TexturePackId {
    #[default]
    Replete,
    Meadow,
    Twilight,
}

impl TexturePackId {
    pub const fn all() -> [Self; 3] {
        [Self::Replete, Self::Meadow, Self::Twilight]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Replete => "Replete",
            Self::Meadow => "Meadow",
            Self::Twilight => "Twilight",
        }
    }

    const fn directory(self) -> &'static str {
        match self {
            Self::Replete => "replete",
            Self::Meadow => "meadow",
            Self::Twilight => "twilight",
        }
    }

    const fn manifest_source(self) -> &'static str {
        match self {
            Self::Replete => include_str!("../../assets/voxel/packs/replete/pack.ron"),
            Self::Meadow => include_str!("../../assets/voxel/packs/meadow/pack.ron"),
            Self::Twilight => include_str!("../../assets/voxel/packs/twilight/pack.ron"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct BlockTextureConfig {
    top: u32,
    side: u32,
    bottom: u32,
    tint_top: (f32, f32, f32),
    tint_side: (f32, f32, f32),
    tint_bottom: (f32, f32, f32),
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct BlockTextures {
    grass: BlockTextureConfig,
    dirt: BlockTextureConfig,
    stone: BlockTextureConfig,
    sand: BlockTextureConfig,
    snow: BlockTextureConfig,
    wood: BlockTextureConfig,
    leaves: BlockTextureConfig,
}

#[derive(Debug, Deserialize)]
struct TexturePackManifest {
    name: String,
    texture: String,
    blocks: BlockTextures,
}

#[derive(Resource, Clone, Debug)]
pub(crate) struct TexturePack {
    pub(crate) texture_path: String,
    blocks: BlockTextures,
}

impl TexturePack {
    pub(crate) fn load(id: TexturePackId) -> Self {
        let manifest =
            ron::from_str::<TexturePackManifest>(id.manifest_source()).unwrap_or_else(|error| {
                warn!("Unable to load texture pack {}: {error}", id.label());
                ron::from_str(TexturePackId::Replete.manifest_source())
                    .expect("the built-in Replete texture pack manifest must be valid")
            });
        debug!("Loaded texture pack {} ({})", id.label(), manifest.name);
        Self {
            texture_path: format!("voxel/packs/{}/{}", id.directory(), manifest.texture),
            blocks: manifest.blocks,
        }
    }

    fn config(&self, kind: VoxelKind) -> Option<&BlockTextureConfig> {
        match kind {
            VoxelKind::Grass => Some(&self.blocks.grass),
            VoxelKind::Dirt => Some(&self.blocks.dirt),
            VoxelKind::Stone => Some(&self.blocks.stone),
            VoxelKind::Sand => Some(&self.blocks.sand),
            VoxelKind::Snow => Some(&self.blocks.snow),
            VoxelKind::Wood => Some(&self.blocks.wood),
            VoxelKind::Leaves => Some(&self.blocks.leaves),
            VoxelKind::Air | VoxelKind::Water => None,
        }
    }

    pub(crate) fn texture_layer(self: &Self, kind: VoxelKind, axis: usize, positive: bool) -> u32 {
        let config = self
            .config(kind)
            .expect("opaque voxels have texture configuration");
        if axis == 1 {
            if positive { config.top } else { config.bottom }
        } else {
            config.side
        }
    }

    pub(crate) fn texture_tint(
        self: &Self,
        kind: VoxelKind,
        axis: usize,
        positive: bool,
    ) -> [f32; 3] {
        let config = self
            .config(kind)
            .expect("opaque voxels have texture configuration");
        if axis == 1 {
            if positive {
                [config.tint_top.0, config.tint_top.1, config.tint_top.2]
            } else {
                [
                    config.tint_bottom.0,
                    config.tint_bottom.1,
                    config.tint_bottom.2,
                ]
            }
        } else {
            [config.tint_side.0, config.tint_side.1, config.tint_side.2]
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct VoxelMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub(crate) blocks: Handle<Image>,
}

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

pub(crate) fn block_texture(asset_server: &AssetServer, path: &str) -> Handle<Image> {
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
        .load(path.to_owned())
}

impl Material for VoxelMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/voxel_material.wgsl".into()
    }
}

impl MaterialExtension for WaterMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/water_material.wgsl".into()
    }
}
