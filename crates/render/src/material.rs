#![allow(clippy::needless_arbitrary_self_type)]
use bevy::render::storage::ShaderBuffer;

use super::{
    fog::{FogLut, FogMaterialKey, specialize_fog},
    meshing::ATTRIBUTE_VOXEL,
};
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MeshPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

/// Terrain in the packed `ATTRIBUTE_VOXEL` format, textured from the pack's array.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[type_path = "hollow_render::material"]
#[bind_group_data(FogMaterialKey)]
pub struct VoxelMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub blocks: Handle<Image>,
    /// [`super::texture_pack::TexturePack::cube_face_buffer`]: switching packs swaps it without remeshing.
    #[storage(2, read_only)]
    pub faces: Handle<ShaderBuffer>,
    #[storage(10, read_only)]
    pub animations: Handle<ShaderBuffer>,
    #[storage(11, read_only, visibility(fragment))]
    pub fog: Handle<ShaderBuffer>,
    #[texture(12, visibility(fragment))]
    #[sampler(13, visibility(fragment))]
    pub fog_sky: Option<Handle<Image>>,
    #[texture(14, dimension = "3d", visibility(fragment))]
    #[sampler(15, visibility(fragment))]
    pub fog_aerial: Option<Handle<Image>>,
}

/// Alpha-tested greedy cubes (leaves), with the terrain's bindings and shaders.
///
/// A separate material so it stays out of the camera's depth prepass, like the
/// model materials: otherwise the prepass stores whole cubes, back layers fail the
/// depth test, and the main pass discards the front holes onto the clear color.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[type_path = "hollow_render::material"]
#[bind_group_data(FogMaterialKey)]
pub struct VoxelCutoutMaterial {
    #[texture(0, dimension = "2d_array")]
    #[sampler(1)]
    pub blocks: Handle<Image>,
    #[storage(2, read_only)]
    pub faces: Handle<ShaderBuffer>,
    #[storage(10, read_only)]
    pub animations: Handle<ShaderBuffer>,
    #[storage(11, read_only, visibility(fragment))]
    pub fog: Handle<ShaderBuffer>,
    #[texture(12, visibility(fragment))]
    #[sampler(13, visibility(fragment))]
    pub fog_sky: Option<Handle<Image>>,
    #[texture(14, dimension = "3d", visibility(fragment))]
    #[sampler(15, visibility(fragment))]
    pub fog_aerial: Option<Handle<Image>>,
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
        "embedded://hollow_render/shaders/voxel_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/voxel_material.wesl".into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/voxel_prepass.wesl".into()
    }

    // Still used by the shadow pass, which keeps the leaves' holes.
    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/voxel_prepass.wesl".into()
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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_fog(descriptor, key.bind_group_data.0);
        voxel_vertex_layout(descriptor, layout)?;
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.push("FOG_BEFORE_ATMOSPHERE".into());
        }
        descriptor.vertex.shader_defs.push("VOXEL_CUTOUT".into());
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.push("VOXEL_CUTOUT".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Reflect, ShaderType)]
#[type_path = "hollow_render::material"]
pub struct WaterSettings {
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

/// Translucent water in the packed `ATTRIBUTE_VOXEL` format, lit like a glossy
/// dielectric (roughness 0.08, reflectance 0.5) by `water_material.wesl`.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
#[type_path = "hollow_render::material"]
#[bind_group_data(FogMaterialKey)]
pub struct WaterMaterial {
    #[uniform(0)]
    settings: WaterSettings,
    #[texture(1, dimension = "2d_array")]
    #[sampler(2)]
    still: Option<Handle<Image>>,
    #[storage(11, read_only, visibility(fragment))]
    fog: Handle<ShaderBuffer>,
    #[texture(12, visibility(fragment))]
    #[sampler(13, visibility(fragment))]
    pub fog_sky: Option<Handle<Image>>,
    #[texture(14, dimension = "3d", visibility(fragment))]
    #[sampler(15, visibility(fragment))]
    pub fog_aerial: Option<Handle<Image>>,
}

impl WaterMaterial {
    pub fn new(
        still: Option<Handle<Image>>,
        frames: u32,
        fps: f32,
        surface: bool,
        fog: &FogLut,
    ) -> Self {
        let animation = if still.is_some() {
            Vec4::new(frames as f32, fps, 0.0, 0.0)
        } else {
            Vec4::new(0.0, 0.0, f32::from(surface), 0.0)
        };
        Self {
            settings: WaterSettings {
                animation,
                ..default()
            },
            still,
            fog: fog.0.clone(),
            fog_sky: fog.1.clone(),
            fog_aerial: fog.2.clone(),
        }
    }
}

impl Material for VoxelMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/voxel_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/voxel_material.wesl".into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/voxel_prepass.wesl".into()
    }

    // Bevy only runs it for emulated unclipped shadow depth.
    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/voxel_prepass.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_fog(descriptor, key.bind_group_data.0);
        voxel_vertex_layout(descriptor, layout)?;
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.push("FOG_BEFORE_ATMOSPHERE".into());
        }
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
        "embedded://hollow_render/shaders/water_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/water_material.wesl".into()
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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_fog(descriptor, key.bind_group_data.0);
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
pub struct ModelMaterial {
    pub instance_light: bool,
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
    #[storage(11, read_only, visibility(fragment))]
    pub fog: Handle<ShaderBuffer>,
    #[texture(12, visibility(fragment))]
    #[sampler(13, visibility(fragment))]
    pub fog_sky: Option<Handle<Image>>,
    #[texture(14, dimension = "3d", visibility(fragment))]
    #[sampler(15, visibility(fragment))]
    pub fog_aerial: Option<Handle<Image>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelMaterialKey {
    instance_light: bool,
    blend: ModelBlend,
    breaking: bool,
    filtered_fog: bool,
}
impl From<&ModelMaterial> for ModelMaterialKey {
    fn from(material: &ModelMaterial) -> Self {
        Self {
            instance_light: material.instance_light,
            blend: material.blend,
            breaking: material.cracks.is_some(),
            filtered_fog: material.fog_sky.is_some(),
        }
    }
}
/// How a model material blends its quads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Reflect)]
pub enum ModelBlend {
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
            instance_light: false,
            blocks,
            faces,
            water: settings,
            still: water.still.clone(),
            blend,
            cracks: None,
            special,
            animations,
            fog: water.fog.clone(),
            fog_sky: water.fog_sky.clone(),
            fog_aerial: water.fog_aerial.clone(),
        }
    }
}
impl Material for ModelMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/model_material.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/model_material.wesl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/model_shadow.wesl".into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/model_shadow.wesl".into()
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
        specialize_fog(descriptor, key.bind_group_data.filtered_fog);
        descriptor.vertex.buffers = vec![
            layout
                .0
                .get_layout(&[super::model_mesh::ATTRIBUTE_MODEL.at_shader_location(0)])?,
        ];
        if key.bind_group_data.instance_light {
            descriptor
                .vertex
                .shader_defs
                .push("MODEL_INSTANCE_LIGHT".into());
        }
        let blend = key.bind_group_data.blend;
        descriptor.primitive.cull_mode = if blend == ModelBlend::Translucent {
            None
        } else {
            Some(bevy::render::render_resource::Face::Back)
        };
        if blend == ModelBlend::Translucent
            && !key.bind_group_data.breaking
            && let Some(depth) = &mut descriptor.depth_stencil
        {
            // Reverse Z: keep water behind coplanar cutouts. The slope term
            // covers rasterization differences at grazing angles, where a
            // constant depth offset alone still leaves alternating fragments.
            // Cutout holes retain background depth and still show the water.
            depth.bias.constant = -2;
            depth.bias.slope_scale = -1.0;
        }
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
            if blend != ModelBlend::Translucent && !key.bind_group_data.breaking {
                fragment.shader_defs.push("FOG_BEFORE_ATMOSPHERE".into());
            }
            match blend {
                ModelBlend::Opaque => {}
                ModelBlend::Cutout => fragment.shader_defs.push("MODEL_CUTOUT".into()),
                ModelBlend::Translucent => fragment.shader_defs.push("MODEL_BLEND".into()),
            }
        }
        Ok(())
    }
}

impl From<&VoxelMaterial> for FogMaterialKey {
    fn from(material: &VoxelMaterial) -> Self {
        Self(material.fog_sky.is_some())
    }
}

impl From<&VoxelCutoutMaterial> for FogMaterialKey {
    fn from(material: &VoxelCutoutMaterial) -> Self {
        Self(material.fog_sky.is_some())
    }
}

impl From<&WaterMaterial> for FogMaterialKey {
    fn from(material: &WaterMaterial) -> Self {
        Self(material.fog_sky.is_some())
    }
}
