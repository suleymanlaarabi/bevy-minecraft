//! A small GPU-only snapshot of the atmosphere for the voxel materials. Sharing
//! one buffer avoids duplicating atmosphere textures and bindings per material.

use crate::voxel::VoxelViewer;
use bevy::{
    core_pipeline::{
        core_3d::main_opaque_pass_3d,
        schedule::{Core3d, Core3dSystems},
    },
    pbr::{
        GpuAtmosphereSettings,
        resources::{
            AtmosphereSampler, AtmosphereTextures, AtmosphereTransform, AtmosphereTransforms,
            AtmosphereTransformsOffset, GpuAtmosphere,
        },
    },
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponentPlugin},
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{binding_types::*, *},
        renderer::{RenderContext, RenderDevice, ViewQuery},
        storage::{GpuShaderBuffer, ShaderBuffer},
        view::{ViewUniform, ViewUniformOffset, ViewUniforms},
    },
};

// Keep the dimensions and offsets in sync with fog_lut.wesl and prepare_fog.wesl.
const SKY_WIDTH: usize = 128;
const SKY_HEIGHT: usize = 64;
const AERIAL_SIZE: usize = 32;
const FOG_ENTRIES: usize = 2 + SKY_WIDTH * SKY_HEIGHT + AERIAL_SIZE.pow(3);

#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub(crate) struct FogLut(pub Handle<ShaderBuffer>);

impl FromWorld for FogLut {
    fn from_world(world: &mut World) -> Self {
        let mut buffer = ShaderBuffer::from(vec![[0.0_f32; 4]; FOG_ENTRIES]);
        buffer.label = "voxel atmosphere snapshot".into();
        Self(world.resource_mut::<Assets<ShaderBuffer>>().add(buffer))
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<FogLut>().add_plugins((
        ExtractResourcePlugin::<FogLut>::default(),
        ExtractComponentPlugin::<VoxelViewer>::default(),
    ));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .add_systems(RenderStartup, init_pipeline)
        .add_systems(
            Render,
            prepare_bind_groups.in_set(RenderSystems::PrepareBindGroups),
        )
        .add_systems(
            Core3d,
            update_lut
                // Bevy generates atmosphere LUTs before MainPass. Read them only
                // afterwards, but before any voxel material samples this buffer.
                .in_set(Core3dSystems::MainPass)
                .before(main_opaque_pass_3d),
        );
}

#[derive(Resource)]
struct FogPipeline {
    atmosphere_layout: BindGroupLayoutDescriptor,
    output_layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
) {
    // The indices intentionally match Bevy's atmosphere shader library bindings.
    let atmosphere_layout = BindGroupLayoutDescriptor::new(
        "voxel fog atmosphere",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, uniform_buffer::<GpuAtmosphere>(true)),
                (1, uniform_buffer::<GpuAtmosphereSettings>(true)),
                (2, uniform_buffer::<AtmosphereTransform>(true)),
                (3, uniform_buffer::<ViewUniform>(true)),
                (
                    10,
                    texture_2d(TextureSampleType::Float { filterable: true }),
                ),
                (
                    11,
                    texture_3d(TextureSampleType::Float { filterable: true }),
                ),
                (12, sampler(SamplerBindingType::Filtering)),
            ),
        ),
    );
    let output_layout = BindGroupLayoutDescriptor::new(
        "voxel fog snapshot",
        &BindGroupLayoutEntries::single(ShaderStages::COMPUTE, storage_buffer::<Vec<Vec4>>(false)),
    );
    let rgb_transmittance = device
        .features()
        .contains(WgpuFeatures::DUAL_SOURCE_BLENDING);
    let pipeline = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("prepare voxel fog".into()),
        layout: vec![atmosphere_layout.clone(), output_layout.clone()],
        shader: assets.load("shaders/prepare_fog.wesl"),
        shader_defs: if rgb_transmittance {
            vec!["RGB_TRANSMITTANCE".into()]
        } else {
            Vec::new()
        },
        ..default()
    });
    commands.insert_resource(FogPipeline {
        atmosphere_layout,
        output_layout,
        pipeline,
    });
}

#[derive(Component)]
struct FogBindGroups {
    atmosphere: BindGroup,
    output: BindGroup,
}

fn prepare_bind_groups(
    mut commands: Commands,
    pipeline: Res<FogPipeline>,
    lut: Res<FogLut>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    atmospheres: Res<ComponentUniforms<GpuAtmosphere>>,
    settings: Res<ComponentUniforms<GpuAtmosphereSettings>>,
    transforms: Option<Res<AtmosphereTransforms>>,
    views: Res<ViewUniforms>,
    sampler: Option<Res<AtmosphereSampler>>,
    cameras: Query<(Entity, &AtmosphereTextures), With<VoxelViewer>>,
) {
    let (Some(transforms), Some(sampler)) = (transforms, sampler) else {
        return;
    };
    let (Some(buffer), Some(atmosphere), Some(settings), Some(transforms), Some(view)) = (
        buffers.get(&lut.0),
        atmospheres.uniforms().binding(),
        settings.uniforms().binding(),
        transforms.uniforms().binding(),
        views.uniforms.binding(),
    ) else {
        return;
    };
    for (entity, textures) in &cameras {
        let atmosphere = device.create_bind_group(
            "voxel fog atmosphere",
            &cache.get_bind_group_layout(&pipeline.atmosphere_layout),
            &BindGroupEntries::with_indices((
                (0, atmosphere.clone()),
                (1, settings.clone()),
                (2, transforms.clone()),
                (3, view.clone()),
                (10, &textures.sky_view_lut.default_view),
                (11, &textures.aerial_view_lut.default_view),
                (12, &**sampler),
            )),
        );
        let output = device.create_bind_group(
            "voxel fog snapshot",
            &cache.get_bind_group_layout(&pipeline.output_layout),
            &BindGroupEntries::single(buffer.buffer.as_entire_binding()),
        );
        commands
            .entity(entity)
            .insert(FogBindGroups { atmosphere, output });
    }
}

fn update_lut(
    camera: ViewQuery<(
        &FogBindGroups,
        &DynamicUniformIndex<GpuAtmosphere>,
        &DynamicUniformIndex<GpuAtmosphereSettings>,
        &AtmosphereTransformsOffset,
        &ViewUniformOffset,
        &VoxelViewer,
    )>,
    pipeline: Res<FogPipeline>,
    cache: Res<PipelineCache>,
    mut context: RenderContext,
) {
    let Some(compute) = cache.get_compute_pipeline(pipeline.pipeline) else {
        return;
    };
    let (groups, atmosphere, settings, transforms, view, _) = camera.into_inner();
    let mut pass = context
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("prepare voxel fog"),
            ..default()
        });
    pass.set_pipeline(compute);
    pass.set_bind_group(
        0,
        &groups.atmosphere,
        &[
            atmosphere.index(),
            settings.index(),
            transforms.index(),
            view.offset,
        ],
    );
    pass.set_bind_group(1, &groups.output, &[]);
    pass.dispatch_workgroups((AERIAL_SIZE.pow(3) as u32).div_ceil(64), 1, 1);
}
