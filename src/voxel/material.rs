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

fn default_texture_layers() -> u32 {
    9
}

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TexturePackId {
    #[default]
    Faithful,
    Replete,
    Meadow,
    Twilight,
}

impl TexturePackId {
    pub const fn all() -> [Self; 4] {
        [Self::Faithful, Self::Replete, Self::Meadow, Self::Twilight]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Faithful => "Faithful 32x",
            Self::Replete => "Replete",
            Self::Meadow => "Meadow",
            Self::Twilight => "Twilight",
        }
    }

    const fn directory(self) -> &'static str {
        match self {
            Self::Faithful => "faithful",
            Self::Replete => "replete",
            Self::Meadow => "meadow",
            Self::Twilight => "twilight",
        }
    }

    const fn manifest_source(self) -> &'static str {
        match self {
            Self::Faithful => include_str!("../../assets/voxel/packs/faithful/pack.ron"),
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
    #[serde(default)]
    deepslate: Option<BlockTextureConfig>,
    #[serde(default)]
    gravel: Option<BlockTextureConfig>,
    #[serde(default)]
    coal_ore: Option<BlockTextureConfig>,
    #[serde(default)]
    iron_ore: Option<BlockTextureConfig>,
    #[serde(default)]
    copper_ore: Option<BlockTextureConfig>,
    #[serde(default)]
    gold_ore: Option<BlockTextureConfig>,
    #[serde(default)]
    redstone_ore: Option<BlockTextureConfig>,
    #[serde(default)]
    diamond_ore: Option<BlockTextureConfig>,
    #[serde(default)]
    emerald_ore: Option<BlockTextureConfig>,
    #[serde(default)]
    lapis_ore: Option<BlockTextureConfig>,
}

#[derive(Debug, Deserialize)]
struct TexturePackManifest {
    name: String,
    texture: String,
    #[serde(default = "default_texture_layers")]
    layers: u32,
    blocks: BlockTextures,
}

#[derive(Resource, Clone, Debug)]
pub(crate) struct TexturePack {
    pub(crate) texture_path: String,
    pub(crate) layers: u32,
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
            layers: manifest.layers,
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
            VoxelKind::Deepslate => {
                Some(self.blocks.deepslate.as_ref().unwrap_or(&self.blocks.stone))
            }
            VoxelKind::Gravel => Some(self.blocks.gravel.as_ref().unwrap_or(&self.blocks.stone)),
            VoxelKind::CoalOre => Some(self.blocks.coal_ore.as_ref().unwrap_or(&self.blocks.stone)),
            VoxelKind::IronOre => Some(self.blocks.iron_ore.as_ref().unwrap_or(&self.blocks.stone)),
            VoxelKind::CopperOre => Some(
                self.blocks
                    .copper_ore
                    .as_ref()
                    .unwrap_or(&self.blocks.stone),
            ),
            VoxelKind::GoldOre => Some(self.blocks.gold_ore.as_ref().unwrap_or(&self.blocks.stone)),
            VoxelKind::RedstoneOre => Some(
                self.blocks
                    .redstone_ore
                    .as_ref()
                    .unwrap_or(&self.blocks.stone),
            ),
            VoxelKind::DiamondOre => Some(
                self.blocks
                    .diamond_ore
                    .as_ref()
                    .unwrap_or(&self.blocks.stone),
            ),
            VoxelKind::EmeraldOre => Some(
                self.blocks
                    .emerald_ore
                    .as_ref()
                    .unwrap_or(&self.blocks.stone),
            ),
            VoxelKind::LapisOre => {
                Some(self.blocks.lapis_ore.as_ref().unwrap_or(&self.blocks.stone))
            }
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

pub(crate) fn block_texture(asset_server: &AssetServer, path: &str, layers: u32) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(move |settings: &mut ImageLoaderSettings| {
            settings.array_layout = Some(ImageArrayLayout::RowCount { rows: layers });
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
        "shaders/voxel_material.wesl".into()
    }
}

impl MaterialExtension for WaterMaterialExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/water_material.wesl".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        app::PluginsState,
        camera::RenderTarget,
        material::descriptor::PipelineDescriptor,
        render::{
            RenderApp, RenderPlugin,
            pipelined_rendering::PipelinedRenderingPlugin,
            render_resource::{CachedPipelineState, PipelineCache, TextureFormat},
        },
        window::ExitCondition,
        winit::WinitPlugin,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn built_in_packs_have_valid_layers_and_assets() {
        for id in TexturePackId::all() {
            let pack = TexturePack::load(id);
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets")
                    .join(&pack.texture_path)
                    .is_file()
            );
            for kind in VoxelKind::ITEMIZABLE {
                for axis in 0..3 {
                    for positive in [false, true] {
                        assert!(pack.texture_layer(kind, axis, positive) < pack.layers);
                    }
                }
            }
        }
        assert_eq!(TexturePackId::default(), TexturePackId::Faithful);
    }

    #[test]
    #[ignore = "requires a Vulkan/graphics adapter; runs without creating a window"]
    fn terrain_and_water_gpu_pipelines_compile_with_and_without_shadows() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<WinitPlugin>()
                .disable::<PipelinedRenderingPlugin>(),
        )
        .add_plugins((
            MaterialPlugin::<VoxelMaterial>::default(),
            MaterialPlugin::<WaterMaterial>::default(),
        ));
        let deadline = Instant::now() + Duration::from_secs(60);
        while app.plugins_state() != PluginsState::Ready {
            assert!(
                Instant::now() < deadline,
                "renderer initialization timed out"
            );
            bevy::tasks::tick_global_task_pools_on_main_thread();
            std::thread::sleep(Duration::from_millis(5));
        }
        app.finish();
        app.cleanup();
        let terrain_shader: Handle<Shader> = app
            .world()
            .resource::<AssetServer>()
            .load("shaders/voxel_material.wesl");
        let water_shader: Handle<Shader> = app
            .world()
            .resource::<AssetServer>()
            .load("shaders/water_material.wesl");
        let texture = block_texture(
            app.world().resource::<AssetServer>(),
            "voxel/packs/faithful/blocks_array.png",
            TexturePack::load(TexturePackId::Faithful).layers,
        );
        let terrain = app
            .world_mut()
            .resource_mut::<Assets<VoxelMaterial>>()
            .add(VoxelMaterial { blocks: texture });
        let water = app
            .world_mut()
            .resource_mut::<Assets<WaterMaterial>>()
            .add(WaterMaterial {
                base: StandardMaterial {
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                },
                extension: default(),
            });
        let pack = TexturePack::load(TexturePackId::default());
        let block = super::super::meshing::build_block_item_mesh(VoxelKind::Stone, &pack);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(block);
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(terrain),
            Transform::from_xyz(-0.7, 0.0, 0.0),
        ));
        app.world_mut().spawn((
            Mesh3d(mesh),
            MeshMaterial3d(water),
            Transform::from_xyz(0.7, 0.0, 0.0),
        ));
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    128,
                    128,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                RenderTarget::Image(target.into()),
                Transform::from_xyz(0.0, 1.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
                bevy::pbr::DistanceFog::default(),
                Msaa::Sample4,
            ))
            .id();
        let light = app
            .world_mut()
            .spawn((
                DirectionalLight {
                    shadow_maps_enabled: true,
                    ..default()
                },
                Transform::from_xyz(2.0, 3.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
            ))
            .id();
        for shadows in [true, false] {
            app.world_mut()
                .get_mut::<DirectionalLight>(light)
                .unwrap()
                .shadow_maps_enabled = shadows;
            app.world_mut().entity_mut(camera).insert(if shadows {
                Msaa::Sample4
            } else {
                Msaa::Off
            });
            let previous_count = app
                .sub_app(RenderApp)
                .world()
                .resource::<PipelineCache>()
                .pipelines()
                .count();
            loop {
                assert!(
                    Instant::now() < deadline,
                    "material pipelines did not become ready"
                );
                app.update();
                let cache = app.sub_app(RenderApp).world().resource::<PipelineCache>();
                let mut compiled = [false; 2];
                for pipeline in cache.pipelines() {
                    if let CachedPipelineState::Err(error) = &pipeline.state {
                        panic!("GPU pipeline failed: {error}");
                    }
                    if let PipelineDescriptor::RenderPipelineDescriptor(descriptor) =
                        &pipeline.descriptor
                        && let Some(fragment) = &descriptor.fragment
                        && matches!(pipeline.state, CachedPipelineState::Ok(_))
                    {
                        for (index, shader) in
                            [&terrain_shader, &water_shader].into_iter().enumerate()
                        {
                            if fragment.shader == *shader
                                && descriptor.multisample.count == if shadows { 4 } else { 1 }
                            {
                                compiled[index] = true;
                            }
                        }
                    }
                }
                if compiled == [true, true] && cache.pipelines().count() > previous_count {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
