#![allow(clippy::needless_arbitrary_self_type)]

use super::{meshing::ATTRIBUTE_VOXEL, texture_pack::block_texture};
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MeshPipelineKey},
    prelude::*,
    render::{
        render_resource::{
            AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        },
        storage::ShaderBuffer,
    },
    shader::ShaderRef,
};

/// Terrain in the packed `ATTRIBUTE_VOXEL` format, textured from the pack's array.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[type_path = "hollow::voxel::material"]
pub(crate) struct VoxelMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub(crate) blocks: Handle<Image>,
    /// [`super::texture_pack::TexturePack::cube_face_buffer`]: switching packs swaps it without remeshing.
    #[storage(2, read_only)]
    pub(crate) faces: Handle<ShaderBuffer>,
    #[storage(10, read_only)]
    pub(crate) animations: Handle<ShaderBuffer>,
}

/// Alpha-tested greedy cubes (leaves), with the terrain's bindings and shaders.
///
/// A separate material so it stays out of the camera's depth prepass, like the
/// model materials: otherwise the prepass stores whole cubes, back layers fail the
/// depth test, and the main pass discards the front holes onto the clear color.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[type_path = "hollow::voxel::material"]
pub(crate) struct VoxelCutoutMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub(crate) blocks: Handle<Image>,
    #[storage(2, read_only)]
    pub(crate) faces: Handle<ShaderBuffer>,
    #[storage(10, read_only)]
    pub(crate) animations: Handle<ShaderBuffer>,
}

fn voxel_vertex_layout(
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
) -> Result<(), SpecializedMeshPipelineError> {
    // Main pass and shadow prepass read the same packed vertex.
    descriptor.vertex.buffers = vec![
        layout
            .0
            .get_layout(&[ATTRIBUTE_VOXEL.at_shader_location(0)])?,
    ];
    Ok(())
}

impl Material for VoxelCutoutMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/voxel_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/voxel_material.wesl".into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/voxel_prepass.wesl".into()
    }

    // Still used by the shadow pass, which keeps the leaves' holes.
    fn prepass_fragment_shader() -> ShaderRef {
        "shaders/voxel_prepass.wesl".into()
    }

    fn enable_prepass() -> bool {
        false
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Mask(0.5)
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        voxel_vertex_layout(descriptor, layout)?;
        descriptor.vertex.shader_defs.push("VOXEL_CUTOUT".into());
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.push("VOXEL_CUTOUT".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Reflect, ShaderType)]
#[type_path = "hollow::voxel::material"]
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
#[type_path = "hollow::voxel::material"]
pub(crate) struct WaterMaterial {
    #[uniform(0)]
    settings: WaterSettings,
    #[texture(1, dimension = "2d_array")]
    #[sampler(2)]
    still: Option<Handle<Image>>,
}

impl WaterMaterial {
    /// Uses the vanilla flipbook when imported, otherwise the shader's procedural texture.
    pub(crate) fn load(asset_server: &AssetServer, surface_water: bool) -> Self {
        let mut material = Self::default();
        material.settings.animation.z = if surface_water { 1.0 } else { 0.0 };
        if !surface_water && crate::asset_directory().join(WATER_STILL_PATH).is_file() {
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

    // Bevy only runs it for emulated unclipped shadow depth.
    fn prepass_fragment_shader() -> ShaderRef {
        "shaders/voxel_prepass.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        voxel_vertex_layout(descriptor, layout)?;
        // Bevy's prepass defaults to drawing both sides, whereas the forward pass
        // culls back faces. Camera coverage must match so invisible back faces do
        // not reject visible terrain at grazing edges. Only directional shadow
        // views stay two-sided: Bevy keys point and spot shadow views exactly like
        // a perspective camera prepass, so they cull too. Voxel shadow meshes keep
        // faces the sky can light only (`faces_sun`) and support no other caster anyway.
        if !key
            .mesh_key
            .contains(MeshPipelineKey::UNCLIPPED_DEPTH_ORTHO)
        {
            descriptor.primitive.cull_mode = Some(Face::Back);
        }
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

/// Shared texture-array material for native models, with explicit positions/UVs.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(ModelMaterialKey)]
pub(crate) struct ModelMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub blocks: Handle<Image>,
    #[storage(2, read_only)]
    pub faces: Handle<ShaderBuffer>,
    #[uniform(3)]
    pub water: WaterSettings,
    #[texture(4, dimension = "2d_array")]
    #[sampler(5)]
    pub still: Option<Handle<Image>>,
    /// Opaque quads skip the discard and keep early depth tests.
    pub blend: ModelBlend,
    #[texture(6)]
    #[sampler(7)]
    pub cracks: Option<Handle<Image>>,
    #[texture(8, dimension = "2d_array")]
    #[sampler(9)]
    pub special: Option<Handle<Image>>,
    #[storage(10, read_only)]
    pub animations: Handle<ShaderBuffer>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ModelMaterialKey {
    blend: ModelBlend,
    breaking: bool,
}
impl From<&ModelMaterial> for ModelMaterialKey {
    fn from(material: &ModelMaterial) -> Self {
        Self {
            blend: material.blend,
            breaking: material.cracks.is_some(),
        }
    }
}
/// How a model material blends its quads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub(crate) enum ModelBlend {
    Opaque,
    Cutout,
    Translucent,
}

impl ModelMaterial {
    pub fn set_water(&mut self, water: &WaterMaterial) {
        self.water = water.settings;
        self.still = water.still.clone();
    }
    pub fn new(
        blocks: Handle<Image>,
        faces: Handle<ShaderBuffer>,
        water: &WaterMaterial,
        blend: ModelBlend,
        special: Option<Handle<Image>>,
        animations: Handle<ShaderBuffer>,
    ) -> Self {
        let settings = water.settings;
        Self {
            blocks,
            faces,
            water: settings,
            still: water.still.clone(),
            blend,
            cracks: None,
            special,
            animations,
        }
    }
}
impl Material for ModelMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/model_material.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/model_material.wesl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        "shaders/model_shadow.wesl".into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        "shaders/model_shadow.wesl".into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn alpha_mode(&self) -> AlphaMode {
        if self.cracks.is_some() {
            return AlphaMode::Multiply;
        }
        match self.blend {
            ModelBlend::Opaque => AlphaMode::Opaque,
            ModelBlend::Cutout => AlphaMode::Mask(0.5),
            ModelBlend::Translucent => AlphaMode::Blend,
        }
    }
    fn depth_bias(&self) -> f32 {
        if self.cracks.is_some() { 1.0 } else { 0.0 }
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![
            layout
                .0
                .get_layout(&[super::model_mesh::ATTRIBUTE_MODEL.at_shader_location(0)])?,
        ];
        let blend = key.bind_group_data.blend;
        descriptor.primitive.cull_mode = if blend == ModelBlend::Translucent {
            None
        } else {
            Some(bevy::render::render_resource::Face::Back)
        };
        if key.bind_group_data.breaking
            && let Some(fragment) = &mut descriptor.fragment
        {
            descriptor.vertex.shader_defs.push("MODEL_BREAKING".into());
            fragment.shader_defs.push("MODEL_BREAKING".into());
        }
        if key.bind_group_data.breaking
            && let Some(depth) = &mut descriptor.depth_stencil
        {
            // The overlay uses the block's exact geometry. Material::depth_bias
            // only changes transparent sorting; offset rasterized depth as well
            // to keep coplanar cracks stable (Bevy uses reverse Z).
            depth.bias.constant = 1;
            depth.bias.slope_scale = 1.0;
        }
        if let Some(fragment) = &mut descriptor.fragment {
            match blend {
                ModelBlend::Opaque => {}
                ModelBlend::Cutout => fragment.shader_defs.push("MODEL_CUTOUT".into()),
                ModelBlend::Translucent => fragment.shader_defs.push("MODEL_BLEND".into()),
            }
        }
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

    use super::super::texture_pack::{TexturePack, TexturePackId};
    use crate::voxel::VoxelKind;
    use bevy::render::render_resource::TextureFormat;
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
            .add(TexturePack::load(TexturePackId::default()).cube_face_buffer());
        let texture = block_texture(
            app.world().resource::<AssetServer>(),
            &TexturePack::load(TexturePackId::Vanilla).texture_path,
            TexturePack::load(TexturePackId::Vanilla).layers,
        );
        let animations = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(vec![[0_u32; 4]]));
        let terrain = app
            .world_mut()
            .resource_mut::<Assets<VoxelMaterial>>()
            .add(VoxelMaterial {
                blocks: texture,
                faces: faces.clone(),
                animations,
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
