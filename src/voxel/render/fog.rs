//! A small GPU-only snapshot of the atmosphere for the voxel materials. Sharing
//! one buffer avoids duplicating atmosphere textures and bindings per material.

use crate::voxel::VoxelViewer;
use bevy::render::storage::{GpuShaderBuffer, ShaderBuffer};
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::{
        core_3d::main_opaque_pass_3d,
        schedule::{Core3d, Core3dSystems},
    },
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
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
        texture::GpuImage,
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
pub(crate) struct FogLut(
    pub Handle<ShaderBuffer>,
    pub Option<Handle<Image>>,
    pub Option<Handle<Image>>,
);

/// Part of the material specialization key; unsupported devices keep storage sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FogMaterialKey(pub bool);

pub(crate) fn specialize_fog(descriptor: &mut RenderPipelineDescriptor, filtered: bool) {
    if filtered && let Some(fragment) = &mut descriptor.fragment {
        fragment.shader_defs.push("FILTERED_FOG".into());
    }
}

fn initialize_filtered_lut(
    device: Option<Res<RenderDevice>>,
    mut lut: ResMut<FogLut>,
    mut images: ResMut<Assets<Image>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    if !device.is_some_and(|device| device.features().contains(WgpuFeatures::FLOAT32_FILTERABLE)) {
        return;
    }
    let mut texture = |size, dimension, repeat_sky| {
        let mut image = Image::new_fill(
            size,
            dimension,
            &[0; 16],
            TextureFormat::Rgba32Float,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING
            | TextureUsages::STORAGE_BINDING
            | TextureUsages::COPY_DST;
        let mut sampler = ImageSamplerDescriptor::linear();
        if repeat_sky {
            sampler.address_mode_u = ImageAddressMode::Repeat;
        }
        image.sampler = ImageSampler::Descriptor(sampler);
        images.add(image)
    };
    lut.1 = Some(texture(
        Extent3d {
            width: SKY_WIDTH as u32,
            height: SKY_HEIGHT as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        true,
    ));
    lut.2 = Some(texture(
        Extent3d {
            width: AERIAL_SIZE as u32,
            height: AERIAL_SIZE as u32,
            depth_or_array_layers: AERIAL_SIZE as u32,
        },
        TextureDimension::D3,
        false,
    ));
    if let Some(mut buffer) = buffers.get_mut(&lut.0) {
        buffer.data = ShaderBuffer::from(vec![[0.0_f32; 4]; 2]).data;
    }
}

impl FromWorld for FogLut {
    fn from_world(world: &mut World) -> Self {
        let mut buffer = ShaderBuffer::from(vec![[0.0_f32; 4]; FOG_ENTRIES]);
        buffer.label = "voxel atmosphere snapshot".into();
        Self(
            world.resource_mut::<Assets<ShaderBuffer>>().add(buffer),
            None,
            None,
        )
    }
}

pub(super) fn register(app: &mut App) {
    app.add_systems(
        PreStartup,
        initialize_filtered_lut.before(super::assets::prepare_assets),
    );
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
    let filtered = device.features().contains(WgpuFeatures::FLOAT32_FILTERABLE);
    let output_layout = if filtered {
        BindGroupLayoutDescriptor::new(
            "voxel filtered fog snapshot",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    storage_buffer::<Vec<Vec4>>(false),
                    texture_storage_2d(TextureFormat::Rgba32Float, StorageTextureAccess::WriteOnly),
                    texture_storage_3d(TextureFormat::Rgba32Float, StorageTextureAccess::WriteOnly),
                ),
            ),
        )
    } else {
        BindGroupLayoutDescriptor::new(
            "voxel fog snapshot",
            &BindGroupLayoutEntries::single(
                ShaderStages::COMPUTE,
                storage_buffer::<Vec<Vec4>>(false),
            ),
        )
    };
    let rgb_transmittance = device
        .features()
        .contains(WgpuFeatures::DUAL_SOURCE_BLENDING);
    let pipeline = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("prepare voxel fog".into()),
        layout: vec![atmosphere_layout.clone(), output_layout.clone()],
        shader: assets.load("shaders/prepare_fog.wesl"),
        shader_defs: {
            let mut defs = Vec::new();
            if rgb_transmittance {
                defs.push("RGB_TRANSMITTANCE".into());
            }
            if filtered {
                defs.push("FILTERED_FOG".into());
            }
            defs
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
    key: FogBindingKey,
}

#[derive(PartialEq, Eq)]
struct FogBindingKey {
    buffers: [(BufferId, u64, Option<std::num::NonZeroU64>); 5],
    textures: [Option<TextureViewId>; 4],
    sampler: SamplerId,
}

fn buffer_key(
    id: BufferId,
    binding: &BindingResource<'_>,
) -> (BufferId, u64, Option<std::num::NonZeroU64>) {
    let BindingResource::Buffer(binding) = binding else {
        unreachable!("uniform buffer binding");
    };
    (id, binding.offset, binding.size)
}

fn prepare_bind_groups(
    mut commands: Commands,
    pipeline: Res<FogPipeline>,
    lut: Res<FogLut>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    atmospheres: Res<ComponentUniforms<GpuAtmosphere>>,
    settings: Res<ComponentUniforms<GpuAtmosphereSettings>>,
    transforms: Option<Res<AtmosphereTransforms>>,
    views: Res<ViewUniforms>,
    sampler: Option<Res<AtmosphereSampler>>,
    cameras: Query<(Entity, &AtmosphereTextures, Option<&FogBindGroups>), With<VoxelViewer>>,
) {
    let (Some(transforms), Some(sampler)) = (transforms, sampler) else {
        return;
    };
    let uniform_ids = [
        atmospheres.uniforms().buffer().map(Buffer::id),
        settings.uniforms().buffer().map(Buffer::id),
        transforms.uniforms().buffer().map(Buffer::id),
        views.uniforms.buffer().map(Buffer::id),
    ];
    let (Some(buffer), Some(atmosphere), Some(settings), Some(transforms), Some(view)) = (
        buffers.get(&lut.0),
        atmospheres.uniforms().binding(),
        settings.uniforms().binding(),
        transforms.uniforms().binding(),
        views.uniforms.binding(),
    ) else {
        return;
    };
    let filtered = lut.1.is_some();
    let sky = lut.1.as_ref().and_then(|image| images.get(image));
    let aerial = lut.2.as_ref().and_then(|image| images.get(image));
    if filtered && (sky.is_none() || aerial.is_none()) {
        return;
    }
    for (entity, textures, previous) in &cameras {
        let key = FogBindingKey {
            buffers: [
                buffer_key(uniform_ids[0].expect("bound atmosphere"), &atmosphere),
                buffer_key(uniform_ids[1].expect("bound settings"), &settings),
                buffer_key(uniform_ids[2].expect("bound transforms"), &transforms),
                buffer_key(uniform_ids[3].expect("bound view"), &view),
                buffer_key(buffer.buffer.id(), &buffer.buffer.as_entire_binding()),
            ],
            textures: [
                Some(textures.sky_view_lut.default_view.id()),
                Some(textures.aerial_view_lut.default_view.id()),
                sky.map(|image| image.texture_view.id()),
                aerial.map(|image| image.texture_view.id()),
            ],
            sampler: sampler.id(),
        };
        if previous.is_some_and(|previous| previous.key == key) {
            continue;
        }
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
        let output_layout = cache.get_bind_group_layout(&pipeline.output_layout);
        let output = if let (Some(sky), Some(aerial)) = (sky, aerial) {
            device.create_bind_group(
                "voxel filtered fog snapshot",
                &output_layout,
                &BindGroupEntries::sequential((
                    buffer.buffer.as_entire_binding(),
                    &sky.texture_view,
                    &aerial.texture_view,
                )),
            )
        } else {
            device.create_bind_group(
                "voxel fog snapshot",
                &output_layout,
                &BindGroupEntries::single(buffer.buffer.as_entire_binding()),
            )
        };
        commands.entity(entity).insert(FogBindGroups {
            atmosphere,
            output,
            key,
        });
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
