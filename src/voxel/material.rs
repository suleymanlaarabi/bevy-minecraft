#![allow(clippy::needless_arbitrary_self_type)]

use bevy::{
    image::{
        ImageAddressMode, ImageArrayLayout, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::{
        render_resource::{
            AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
            TextureDataOrder, TextureFormat,
        },
        storage::ShaderBuffer,
    },
    shader::ShaderRef,
};
use serde::Deserialize;
use std::collections::HashMap;

use super::{data::VoxelKind, meshing::ATTRIBUTE_VOXEL};

fn default_texture_layers() -> u32 {
    9
}

#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexturePackId {
    Vanilla,
    Faithful,
    Replete,
    Meadow,
    Twilight,
}

impl TexturePackId {
    pub const fn all() -> [Self; 5] {
        [
            Self::Vanilla,
            Self::Faithful,
            Self::Replete,
            Self::Meadow,
            Self::Twilight,
        ]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Vanilla => "Minecraft",
            Self::Faithful => "Faithful 32x",
            Self::Replete => "Replete",
            Self::Meadow => "Meadow",
            Self::Twilight => "Twilight",
        }
    }

    /// Vanilla textures are not redistributable: they exist only after running
    /// scripts/import_vanilla.py. Every other pack ships with the game.
    pub fn is_available(self) -> bool {
        self != Self::Vanilla
            || crate::asset_directory()
                .join(format!("voxel/packs/{}/blocks_array.png", self.directory()))
                .is_file()
    }

    const fn directory(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Faithful => "faithful",
            Self::Replete => "replete",
            Self::Meadow => "meadow",
            Self::Twilight => "twilight",
        }
    }

    const fn manifest_source(self) -> &'static str {
        match self {
            Self::Vanilla => include_str!("../../assets/voxel/packs/vanilla/pack.ron"),
            Self::Faithful => include_str!("../../assets/voxel/packs/faithful/pack.ron"),
            Self::Replete => include_str!("../../assets/voxel/packs/replete/pack.ron"),
            Self::Meadow => include_str!("../../assets/voxel/packs/meadow/pack.ron"),
            Self::Twilight => include_str!("../../assets/voxel/packs/twilight/pack.ron"),
        }
    }
}

/// Minecraft textures when imported, otherwise the bundled Faithful pack.
impl Default for TexturePackId {
    fn default() -> Self {
        if Self::Vanilla.is_available() {
            Self::Vanilla
        } else {
            Self::Faithful
        }
    }
}

fn white() -> (f32, f32, f32) {
    (1.0, 1.0, 1.0)
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct BlockTextureConfig {
    top: u32,
    side: u32,
    bottom: u32,
    #[serde(default = "white")]
    tint_top: (f32, f32, f32),
    #[serde(default = "white")]
    tint_side: (f32, f32, f32),
    #[serde(default = "white")]
    tint_bottom: (f32, f32, f32),
}

/// Textures keyed by `VoxelKind::key`; missing blocks use their fallback chain.
type BlockTextures = HashMap<String, BlockTextureConfig>;

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
    /// Fallbacks resolved at load, indexed by `VoxelKind as usize`; `None` when not opaque.
    blocks: Box<[Option<BlockTextureConfig>]>,
}

/// Blocks still recognizable when drawn with their fallback's texture, tinted by their color.
const TINTED_FALLBACKS: [VoxelKind; 2] = [VoxelKind::Lava, VoxelKind::MagmaBlock];

fn resolve(blocks: &BlockTextures, kind: VoxelKind) -> Option<BlockTextureConfig> {
    if !kind.is_opaque() {
        return None;
    }
    let mut source = kind;
    let mut config = loop {
        if let Some(config) = blocks.get(source.key()) {
            break *config;
        }
        let fallback = source.fallback();
        if fallback == source {
            break blocks[VoxelKind::Stone.key()];
        }
        source = fallback;
    };
    if source != kind && TINTED_FALLBACKS.contains(&kind) {
        let [r, g, b, _] = kind.color();
        let [base_r, base_g, base_b, _] = source.color();
        let tint = (
            (r / base_r).min(2.0),
            (g / base_g).min(2.0),
            (b / base_b).min(2.0),
        );
        (config.tint_top, config.tint_side, config.tint_bottom) = (tint, tint, tint);
    }
    Some(config)
}

impl TexturePack {
    pub(crate) fn load(id: TexturePackId) -> Self {
        Self::try_load(id).unwrap_or_else(|error| {
            // Vanilla textures are not redistributable: they exist only after
            // running scripts/import_vanilla.py.
            warn!("Unable to load texture pack {}: {error}", id.label());
            // Bundled and checked by tests: no filesystem access can make this fail.
            Self::try_load(TexturePackId::Faithful)
                .expect("the built-in Faithful texture pack must be valid")
        })
    }

    fn try_load(id: TexturePackId) -> Result<Self, String> {
        let manifest = ron::from_str::<TexturePackManifest>(id.manifest_source())
            .map_err(|error| error.to_string())?;
        if !manifest.blocks.contains_key(VoxelKind::Stone.key()) {
            return Err("no \"stone\" entry, the fallback of every block".into());
        }
        let texture_path = format!("voxel/packs/{}/{}", id.directory(), manifest.texture);
        if !id.is_available() {
            return Err(format!("missing {texture_path}"));
        }
        debug!("Loaded texture pack {} ({})", id.label(), manifest.name);
        Ok(Self {
            texture_path,
            layers: manifest.layers,
            blocks: VoxelKind::ALL
                .iter()
                .map(|&kind| resolve(&manifest.blocks, kind))
                .collect(),
        })
    }

    fn config(&self, kind: VoxelKind) -> Option<&BlockTextureConfig> {
        self.blocks[kind as usize].as_ref()
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

    /// `(tint, layer)` of every block face, indexed by `kind * 3 + slot` with slots
    /// top, side and bottom, as read by `voxel_material.wesl`.
    fn face_table(&self) -> Vec<[f32; 4]> {
        VoxelKind::ALL
            .iter()
            .flat_map(|&kind| {
                [(1, true), (0, true), (1, false)].map(|(axis, positive)| {
                    if self.config(kind).is_none() {
                        return [0.0; 4];
                    }
                    let [r, g, b] = self.texture_tint(kind, axis, positive);
                    [r, g, b, self.texture_layer(kind, axis, positive) as f32]
                })
            })
            .collect()
    }

    pub(crate) fn face_buffer(&self) -> ShaderBuffer {
        ShaderBuffer::from(self.face_table())
    }
}

/// Terrain in the packed `ATTRIBUTE_VOXEL` format, textured from the pack's array.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub(crate) struct VoxelMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub(crate) blocks: Handle<Image>,
    /// [`TexturePack::face_buffer`]: switching packs swaps it without remeshing.
    #[storage(2, read_only)]
    pub(crate) faces: Handle<ShaderBuffer>,
}

#[derive(Clone, Copy, Debug, Reflect, ShaderType)]
pub(crate) struct WaterSettings {
    /// Linear water tint in RGB.
    tint: Vec4,
    /// Linear sky color reflected at grazing angles in RGB, maximum reflection in A.
    sky_color: Vec4,
    /// Opacity of shallow water, deep water, the underside and grazing views.
    alpha: Vec4,
    /// Flipbook frame count in X (0 without a texture) and frames per second in Y.
    animation: Vec4,
}

impl Default for WaterSettings {
    fn default() -> Self {
        Self {
            // Vanilla's default biome water color, #3F76E4, multiplied by the grey flipbook.
            tint: Color::srgb_u8(63, 118, 228).to_linear().to_vec4(),
            sky_color: Color::srgb_u8(148, 195, 255)
                .to_linear()
                .with_alpha(0.25)
                .to_vec4(),
            alpha: Vec4::new(0.55, 0.8, 0.6, 0.9),
            animation: Vec4::ZERO,
        }
    }
}

/// Vanilla's still water flipbook, imported by scripts/import_vanilla.py.
const WATER_STILL_PATH: &str = "voxel/packs/vanilla/water_still.png";
const WATER_STILL_FRAMES: u32 = 32;
/// Vanilla advances the flipbook every 2 ticks.
const WATER_STILL_FPS: f32 = 10.0;

/// Translucent water in the packed `ATTRIBUTE_VOXEL` format, lit like a glossy
/// dielectric (roughness 0.08, reflectance 0.5) by `water_material.wesl`.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub(crate) struct WaterMaterial {
    #[uniform(0)]
    settings: WaterSettings,
    #[texture(1, dimension = "2d_array")]
    #[sampler(2)]
    still: Option<Handle<Image>>,
}

impl WaterMaterial {
    /// Uses the vanilla flipbook when imported, otherwise the shader's procedural texture.
    pub(crate) fn load(asset_server: &AssetServer) -> Self {
        let mut material = Self::default();
        if crate::asset_directory().join(WATER_STILL_PATH).is_file() {
            material.still = Some(block_texture(
                asset_server,
                WATER_STILL_PATH,
                WATER_STILL_FRAMES,
            ));
            material.settings.animation =
                Vec4::new(WATER_STILL_FRAMES as f32, WATER_STILL_FPS, 0.0, 0.0);
        }
        material
    }
}

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
                // Blends between the levels of `block_mipmaps`, so no mip seams show.
                mipmap_filter: ImageFilterMode::Linear,
                ..default()
            });
        })
        .load(path.to_owned())
}

/// Box-filtered mip chain of a pixel-art array texture, as `(data, level count)` in
/// layer-major order, or `None` when the image already has one or is unsupported.
///
/// Without it, distant terrain samples full-resolution texels: they shimmer and
/// scatter texture reads across the cache. Colors are averaged in linear space and
/// weighted by alpha. The terrain shader shades a texel as `mix(dark, rgb, alpha)`,
/// which is linear in `(rgb * alpha, alpha)`, so each level is exactly the average of
/// the shaded texels it covers and leaf cutouts keep their brightness at a distance.
fn block_mipmaps(image: &Image) -> Option<(Vec<u8>, u32)> {
    let descriptor = &image.texture_descriptor;
    let srgb = match descriptor.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return None,
    };
    let (width, height) = (descriptor.size.width, descriptor.size.height);
    if descriptor.mip_level_count != 1 || !width.is_power_of_two() || !height.is_power_of_two() {
        return None;
    }
    let levels = width.min(height).ilog2() + 1;
    let data = image.data.as_ref()?;
    let layer_bytes = (width * height * 4) as usize;
    if levels == 1 || data.len() != layer_bytes * descriptor.size.depth_or_array_layers as usize {
        return None;
    }

    let decode: [f32; 256] = std::array::from_fn(|value| {
        let value = value as f32 / 255.0;
        if !srgb {
            value
        } else if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    let encode = |value: f32| {
        let value = value.clamp(0.0, 1.0);
        let value = if !srgb {
            value
        } else if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (value * 255.0).round() as u8
    };

    let mut mipmapped = Vec::with_capacity(data.len() * 4 / 3 + 4);
    for layer in data.chunks_exact(layer_bytes) {
        mipmapped.extend_from_slice(layer);
        // Premultiplied linear color: plain averages are then correct.
        let mut texels: Vec<[f32; 4]> = layer
            .chunks_exact(4)
            .map(|texel| {
                let alpha = f32::from(texel[3]) / 255.0;
                [
                    decode[texel[0] as usize] * alpha,
                    decode[texel[1] as usize] * alpha,
                    decode[texel[2] as usize] * alpha,
                    alpha,
                ]
            })
            .collect();
        let (mut level_width, mut level_height) = (width as usize, height as usize);
        for _ in 1..levels {
            let (next_width, next_height) = (level_width / 2, level_height / 2);
            texels = (0..next_width * next_height)
                .map(|index| {
                    let (x, y) = (index % next_width * 2, index / next_width * 2);
                    let mut sum = [0.0; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let texel = texels[x + dx + (y + dy) * level_width];
                        for channel in 0..4 {
                            sum[channel] += texel[channel] * 0.25;
                        }
                    }
                    sum
                })
                .collect();
            mipmapped.extend(texels.iter().flat_map(|&[r, g, b, alpha]| {
                let color = |value: f32| encode(if alpha > 0.0 { value / alpha } else { 0.0 });
                [color(r), color(g), color(b), (alpha * 255.0).round() as u8]
            }));
            (level_width, level_height) = (next_width, next_height);
        }
    }
    Some((mipmapped, levels))
}

/// Gives the terrain's block texture its mip chain once it loads, including after a
/// texture pack switch. Runs only on image or material asset events.
pub(crate) fn generate_block_mipmaps(
    assets: Option<Res<super::rebuild::VoxelAssets>>,
    materials: Res<Assets<VoxelMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(material) = assets.and_then(|assets| materials.get(&assets.terrain)) else {
        return;
    };
    // Checked before `get_mut`, which would re-upload the image and wake this system.
    let Some((data, levels)) = images.get(&material.blocks).and_then(block_mipmaps) else {
        return;
    };
    if let Some(mut image) = images.get_mut(&material.blocks) {
        image.data = Some(data);
        image.data_order = TextureDataOrder::LayerMajor;
        image.texture_descriptor.mip_level_count = levels;
    }
}

impl Material for VoxelMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/voxel_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/voxel_material.wesl".into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/voxel_prepass.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Main pass and shadow prepass read the same packed vertex.
        descriptor.vertex.buffers = vec![
            layout
                .0
                .get_layout(&[ATTRIBUTE_VOXEL.at_shader_location(0)])?,
        ];
        Ok(())
    }
}

impl Material for WaterMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/water_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/water_material.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    // Water neither casts shadows nor writes depth before the main pass.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![
            layout
                .0
                .get_layout(&[ATTRIBUTE_VOXEL.at_shader_location(0)])?,
        ];
        // Both sides render: the surface is also seen from below.
        descriptor.primitive.cull_mode = None;
        Ok(())
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
            render_resource::{CachedPipelineState, PipelineCache},
        },
        window::ExitCondition,
        winit::WinitPlugin,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn block_mipmaps_average_shaded_texels_per_layer() {
        use bevy::render::render_resource::{Extent3d, TextureDimension};
        // Two 2x2 layers: opaque red with one transparent texel, then opaque white.
        let red = [255, 0, 0, 255];
        let data = [
            red,
            red,
            red,
            [0, 0, 0, 0],
            [255; 4],
            [255; 4],
            [255; 4],
            [255; 4],
        ];
        let image = Image::new(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 2,
            },
            TextureDimension::D2,
            data.concat(),
            TextureFormat::Rgba8UnormSrgb,
            default(),
        );
        let (mipmapped, levels) = block_mipmaps(&image).unwrap();
        assert_eq!(levels, 2);
        // Layer-major: each layer is followed by its own 1x1 level.
        assert_eq!(mipmapped.len(), 2 * (16 + 4));
        assert_eq!(&mipmapped[16..20], &[255, 0, 0, 191]);
        assert_eq!(&mipmapped[36..40], &[255; 4]);
    }

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
        assert!(TexturePackId::default().is_available());
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
        let faces = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(TexturePack::load(TexturePackId::default()).face_buffer());
        let texture = block_texture(
            app.world().resource::<AssetServer>(),
            &TexturePack::load(TexturePackId::Vanilla).texture_path,
            TexturePack::load(TexturePackId::Vanilla).layers,
        );
        let terrain = app
            .world_mut()
            .resource_mut::<Assets<VoxelMaterial>>()
            .add(VoxelMaterial {
                blocks: texture,
                faces: faces.clone(),
            });
        let water = app
            .world_mut()
            .resource_mut::<Assets<WaterMaterial>>()
            .add(WaterMaterial::default());
        let block = super::super::meshing::build_block_item_mesh(VoxelKind::Stone);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(block);
        app.world_mut().spawn((
            Mesh3d(mesh),
            MeshMaterial3d(terrain),
            Transform::from_xyz(-0.7, 0.0, 0.0),
        ));
        // Any packed cube exercises the water pipeline's vertex layout.
        let water_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
            super::super::meshing::build_block_item_mesh(VoxelKind::Stone),
        );
        app.world_mut().spawn((
            Mesh3d(water_mesh),
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
