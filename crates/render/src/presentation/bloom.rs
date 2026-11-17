//! Bloom for directly presented views: Bevy's blur pyramid without its final pass.
//!
//! Bevy's final bloom pass blends the pyramid's first level into the whole scene: a
//! full-screen read and write of the scene texture. The present pass reads the scene
//! anyway, so it adds the bloom there (`present_scene.wesl`). The pyramid itself is
//! Bevy's: same shader, sizes, filters and blend factors. Bevy's pipeline types are
//! private, so its passes are rebuilt here around the same shader.
use bevy::{
    core_pipeline::FullscreenShader,
    ecs::entity::EntityHashMap,
    post_process::bloom::{Bloom, BloomCompositeMode},
    prelude::*,
    render::{
        diagnostic::RecordDiagnostics,
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer_sized},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::ViewTarget,
    },
    shader::ShaderDefVal,
};
use std::num::NonZero;

use super::present::DirectPresent;

const FORMAT: TextureFormat = TextureFormat::Rg11b10Ufloat;
/// Bevy's `BloomUniforms`: threshold terms, viewport, scale and aspect, std140.
type Uniforms = [f32; 12];

/// Prefilter, uniform scale, composite mode.
type PipelineKey = (bool, bool, BloomCompositeMode);

#[derive(Resource)]
pub(super) struct BloomPasses {
    shader: Handle<Shader>,
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    /// First downsample, downsample and upsample pipelines.
    pipelines: Vec<(PipelineKey, [CachedRenderPipelineId; 3])>,
    pyramids: EntityHashMap<Pyramid>,
}

pub(super) struct Pyramid {
    size: UVec2,
    uniforms: Uniforms,
    params: [f32; 4],
    pipelines: [CachedRenderPipelineId; 3],
    /// Blend factor of each upsample into the level above it, by source level.
    blends: Vec<f32>,
    views: Vec<TextureView>,
    /// Bind group sampling each level.
    reads: Vec<BindGroup>,
    uniform: Buffer,
    /// Blend factor and scale of the composite in the present pass.
    composite: Buffer,
}

impl Pyramid {
    pub(super) fn first_level(&self) -> &TextureView {
        &self.views[0]
    }

    pub(super) fn composite(&self) -> &Buffer {
        &self.composite
    }
}

impl BloomPasses {
    pub(super) fn new(world: &World) -> Self {
        Self {
            shader: world
                .resource::<AssetServer>()
                .load("embedded://bevy_post_process/bloom/bloom.wesl"),
            layout: BindGroupLayoutDescriptor::new(
                "present bloom layout",
                &BindGroupLayoutEntries::sequential(
                    ShaderStages::FRAGMENT,
                    (
                        texture_2d(TextureSampleType::Float { filterable: true }),
                        sampler(SamplerBindingType::Filtering),
                        uniform_buffer_sized(false, NonZero::new(size_of::<Uniforms>() as u64)),
                    ),
                ),
            ),
            sampler: world
                .resource::<RenderDevice>()
                .create_sampler(&SamplerDescriptor {
                    min_filter: FilterMode::Linear,
                    mag_filter: FilterMode::Linear,
                    address_mode_u: AddressMode::ClampToEdge,
                    address_mode_v: AddressMode::ClampToEdge,
                    ..default()
                }),
            pipelines: Vec::new(),
            pyramids: EntityHashMap::default(),
        }
    }

    pub(super) fn pyramid(&self, view: Entity) -> Option<&Pyramid> {
        self.pyramids.get(&view)
    }

    /// Drops the pyramids of views no longer presented with bloom.
    pub(super) fn retain(&mut self, mut keep: impl FnMut(Entity) -> bool) {
        self.pyramids.retain(|view, _| keep(*view));
    }

    /// Prepares the view's pyramid for a viewport of `size`. False until its
    /// pipelines are compiled: the view keeps Bevy's bloom meanwhile.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        view: Entity,
        size: UVec2,
        bloom: &Bloom,
        device: &RenderDevice,
        queue: &RenderQueue,
        cache: &PipelineCache,
        fullscreen: &FullscreenShader,
    ) -> bool {
        let key = (
            bloom.prefilter.threshold > 0.0,
            bloom.scale == Vec2::ONE,
            bloom.composite_mode,
        );
        let pipelines = match self.pipelines.iter().find(|(known, _)| *known == key) {
            Some(&(_, ids)) => ids,
            None => {
                let ids = self.queue_pipelines(key, cache, fullscreen);
                self.pipelines.push((key, ids));
                ids
            }
        };
        if pipelines
            .iter()
            .any(|&id| cache.get_render_pipeline(id).is_none())
        {
            self.pyramids.remove(&view);
            return false;
        }

        let threshold = bloom.prefilter.threshold;
        let knee = threshold * bloom.prefilter.threshold_softness.clamp(0.0, 1.0);
        // A direct view covers its whole target.
        let uniforms = [
            threshold,
            threshold - knee,
            2.0 * knee,
            0.25 / (knee + 0.00001),
            0.0,
            0.0,
            1.0,
            1.0,
            bloom.scale.x,
            bloom.scale.y,
            size.x as f32 / size.y as f32,
            0.0,
        ];
        let levels = bloom.max_mip_dimension.ilog2().max(2) - 1;
        let max_level = (levels - 1) as f32;
        let blends: Vec<f32> = (0..levels)
            .map(|level| blend_factor(bloom, level as f32, max_level))
            .collect();
        let params = [blends[0], bloom.scale.x, bloom.scale.y, 0.0];
        let texture_size = (size.as_vec2() * (bloom.max_mip_dimension as f32 / size.y as f32))
            .round()
            .as_uvec2()
            .max(UVec2::ONE);

        if let Some(pyramid) = self.pyramids.get_mut(&view)
            && pyramid.size == texture_size
            && pyramid.views.len() == levels as usize
        {
            if pyramid.uniforms != uniforms {
                pyramid.uniforms = uniforms;
                queue.write_buffer(&pyramid.uniform, 0, bytemuck::cast_slice(&uniforms));
            }
            if pyramid.params != params {
                pyramid.params = params;
                queue.write_buffer(&pyramid.composite, 0, bytemuck::cast_slice(&params));
            }
            pyramid.blends = blends;
            pyramid.pipelines = pipelines;
            return true;
        }

        let texture = device.create_texture(&TextureDescriptor {
            label: Some("present bloom"),
            size: Extent3d {
                width: texture_size.x,
                height: texture_size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let views: Vec<TextureView> = (0..levels)
            .map(|level| {
                texture.create_view(&TextureViewDescriptor {
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..default()
                })
            })
            .collect();
        let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("present bloom uniforms"),
            contents: bytemuck::cast_slice(&uniforms),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let composite = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("present bloom composite"),
            contents: bytemuck::cast_slice(&params),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let layout = cache.get_bind_group_layout(&self.layout);
        let reads = views
            .iter()
            .map(|view| {
                device.create_bind_group(
                    "present bloom level",
                    &layout,
                    &BindGroupEntries::sequential((
                        view,
                        &self.sampler,
                        uniform.as_entire_binding(),
                    )),
                )
            })
            .collect();
        self.pyramids.insert(
            view,
            Pyramid {
                size: texture_size,
                uniforms,
                params,
                pipelines,
                blends,
                views,
                reads,
                uniform,
                composite,
            },
        );
        true
    }

    fn queue_pipelines(
        &self,
        (prefilter, uniform_scale, composite): PipelineKey,
        cache: &PipelineCache,
        fullscreen: &FullscreenShader,
    ) -> [CachedRenderPipelineId; 3] {
        let pipeline =
            |label: &'static str, entry: &'static str, first: bool, blend: Option<BlendState>| {
                let mut shader_defs: Vec<ShaderDefVal> = Vec::new();
                if first {
                    shader_defs.push("FIRST_DOWNSAMPLE".into());
                }
                if prefilter {
                    shader_defs.push("USE_THRESHOLD".into());
                }
                if uniform_scale {
                    shader_defs.push("UNIFORM_SCALE".into());
                }
                cache.queue_render_pipeline(RenderPipelineDescriptor {
                    label: Some(label.into()),
                    layout: vec![self.layout.clone()],
                    vertex: fullscreen.to_vertex_state(),
                    fragment: Some(FragmentState {
                        shader: self.shader.clone(),
                        shader_defs,
                        entry_point: Some(entry.into()),
                        targets: vec![Some(ColorTargetState {
                            format: FORMAT,
                            blend,
                            write_mask: ColorWrites::ALL,
                        })],
                        ..default()
                    }),
                    ..default()
                })
            };
        let color = match composite {
            BloomCompositeMode::EnergyConserving => BlendComponent {
                src_factor: BlendFactor::Constant,
                dst_factor: BlendFactor::OneMinusConstant,
                operation: BlendOperation::Add,
            },
            BloomCompositeMode::Additive => BlendComponent {
                src_factor: BlendFactor::Constant,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        };
        [
            pipeline(
                "present bloom first downsample",
                "downsample_first",
                true,
                None,
            ),
            pipeline("present bloom downsample", "downsample", false, None),
            pipeline(
                "present bloom upsample",
                "upsample",
                false,
                Some(BlendState {
                    color,
                    alpha: BlendComponent {
                        src_factor: BlendFactor::Zero,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                }),
            ),
        ]
    }
}

/// Bevy's `compute_blend_factor`: the weight of pyramid level `level` when it is
/// blended into the level above it.
fn blend_factor(bloom: &Bloom, level: f32, max_level: f32) -> f32 {
    let mut boost = (1.0
        - (1.0 - level / max_level).powf(1.0 / (1.0 - bloom.low_frequency_boost_curvature)))
        * bloom.low_frequency_boost;
    let high_pass = 1.0
        - (((level / max_level) - bloom.high_pass_frequency) / bloom.high_pass_frequency)
            .clamp(0.0, 1.0);
    boost *= match bloom.composite_mode {
        BloomCompositeMode::EnergyConserving => 1.0 - bloom.intensity,
        BloomCompositeMode::Additive => 1.0,
    };
    (bloom.intensity + boost) * high_pass
}

/// Downsamples the scene into the pyramid and upsamples it back to its first level.
pub(super) fn render_pyramid(
    view: ViewQuery<&ViewTarget, With<DirectPresent>>,
    passes: Res<BloomPasses>,
    cache: Res<PipelineCache>,
    mut scene_read: Local<Option<((TextureViewId, BufferId), BindGroup)>>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let target = view.into_inner();
    let Some(pyramid) = passes.pyramids.get(&entity) else {
        return;
    };
    let [Some(first), Some(down), Some(up)] =
        pyramid.pipelines.map(|id| cache.get_render_pipeline(id))
    else {
        return;
    };
    let scene = target.main_texture_view();
    let key = (scene.id(), pyramid.uniform.id());
    if scene_read
        .as_ref()
        .is_none_or(|(current, _)| *current != key)
    {
        let bind_group = ctx.render_device().create_bind_group(
            "present bloom scene",
            &cache.get_bind_group_layout(&passes.layout),
            &BindGroupEntries::sequential((
                scene,
                &passes.sampler,
                pyramid.uniform.as_entire_binding(),
            )),
        );
        *scene_read = Some((key, bind_group));
    }
    let Some((_, scene_read)) = scene_read.as_ref() else {
        return;
    };

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(ctx.command_encoder(), "bloom");
    let encoder = ctx.command_encoder();
    let mut draw = |label: &str,
                    target: &TextureView,
                    load: LoadOp<_>,
                    pipeline: &RenderPipeline,
                    read: &BindGroup,
                    blend: Option<f32>| {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, read, &[]);
        if let Some(blend) = blend {
            pass.set_blend_constant(LinearRgba::gray(blend).into());
        }
        pass.draw(0..3, 0..1);
    };
    // Every texel of a level is written before it is read.
    let clear = LoadOp::Clear(default());
    draw(
        "bloom downsample",
        &pyramid.views[0],
        clear,
        first,
        scene_read,
        None,
    );
    for level in 1..pyramid.views.len() {
        draw(
            "bloom downsample",
            &pyramid.views[level],
            clear,
            down,
            &pyramid.reads[level - 1],
            None,
        );
    }
    for level in (1..pyramid.views.len()).rev() {
        draw(
            "bloom upsample",
            &pyramid.views[level - 1],
            LoadOp::Load,
            up,
            &pyramid.reads[level],
            Some(pyramid.blends[level]),
        );
    }
    span.end(ctx.command_encoder());
}
