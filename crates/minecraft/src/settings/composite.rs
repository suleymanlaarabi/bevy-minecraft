//! Scaled rendering presents the scene with one pass: the compositor camera samples
//! the scene camera's tonemapped main texture directly. Bevy's own route copies it
//! three times (to the target image, through a UI quad, then to the window).
use bevy::{
    core_pipeline::{Core2dSystems, FullscreenShader, schedule::Core2d},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        camera::{ExtractedCamera, NormalizedRenderTargetExt},
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{sampler, texture_2d},
            *,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        sync_world::MainEntity,
        texture::{GpuImage, ManualTextureViews, OutputColorAttachment},
        view::{
            ExtractedWindow, ViewTarget, ViewTargetAttachments, prepare_view_attachments,
            prepare_view_targets,
        },
    },
};

/// Camera whose tonemapped main texture is the scene; its own output is skipped.
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
pub(super) struct SceneSource;

/// Window camera that receives the scaled scene below its UI.
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
pub(super) struct SceneCompositor;

#[derive(Resource, Clone)]
struct CompositeShader(Handle<Shader>);

#[derive(Resource)]
struct CompositePipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    vertex: VertexState,
    shader: Handle<Shader>,
    pipeline: Option<(TextureFormat, CachedRenderPipelineId)>,
}

pub(super) fn register(app: &mut App) {
    app.add_plugins((
        ExtractComponentPlugin::<SceneSource>::default(),
        ExtractComponentPlugin::<SceneCompositor>::default(),
    ));
    let shader = app
        .world_mut()
        .resource_mut::<Assets<Shader>>()
        .add(Shader::from_wgsl(
            include_str!("composite.wgsl"),
            "hollow/settings/composite.wgsl",
        ));
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .insert_resource(CompositeShader(shader))
        .add_systems(RenderStartup, init_pipeline)
        .add_systems(
            Render,
            (
                keep_scene_output
                    .in_set(RenderSystems::PrepareViews)
                    .after(prepare_view_attachments)
                    .before(prepare_view_targets),
                queue_pipeline.in_set(RenderSystems::Prepare),
            ),
        )
        .add_systems(
            Core2d,
            composite_scene
                .after(Core2dSystems::MainPass)
                .before(Core2dSystems::PostProcess),
        );
}

fn init_pipeline(
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    shader: Res<CompositeShader>,
    mut commands: Commands,
) {
    commands.insert_resource(CompositePipeline {
        layout: BindGroupLayoutDescriptor::new(
            "scene composite",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        ),
        sampler: device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
        vertex: fullscreen.to_vertex_state(),
        shader: shader.0.clone(),
        pipeline: None,
    });
}

/// Bevy drops the view target of a camera whose output nobody writes. The scene
/// camera skips its output, so its target is registered here to keep it rendering.
fn keep_scene_output(
    cameras: Query<&ExtractedCamera, With<SceneSource>>,
    images: Res<RenderAssets<GpuImage>>,
    manual: Res<ManualTextureViews>,
    windows: Query<(MainEntity, &ExtractedWindow)>,
    mut attachments: ResMut<ViewTargetAttachments>,
) {
    for camera in &cameras {
        let Some(target) = &camera.target else {
            continue;
        };
        if attachments.contains_key(target) {
            continue;
        }
        if let Some((view, format)) = target
            .get_texture_view(&windows, &images, &manual)
            .cloned()
            .zip(target.get_texture_view_format(&windows, &images, &manual))
        {
            attachments.insert(target.clone(), OutputColorAttachment::new(view, format));
        }
    }
}

fn queue_pipeline(
    mut composite: ResMut<CompositePipeline>,
    cache: Res<PipelineCache>,
    views: Query<&ViewTarget, With<SceneCompositor>>,
) {
    let Some(format) = views.iter().next().map(ViewTarget::main_texture_format) else {
        return;
    };
    if composite
        .pipeline
        .is_some_and(|(current, _)| current == format)
    {
        return;
    }
    // A linear main texture encodes on write; an sRGB-compositing one stores
    // encoded values in a plain 8-bit format.
    let encoded = matches!(
        format,
        TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm
    );
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("scene composite".into()),
        layout: vec![composite.layout.clone()],
        vertex: composite.vertex.clone(),
        fragment: Some(FragmentState {
            shader: composite.shader.clone(),
            entry_point: Some(
                if encoded {
                    "fragment_encoded"
                } else {
                    "fragment"
                }
                .into(),
            ),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    composite.pipeline = Some((format, id));
}

fn composite_scene(
    view: ViewQuery<&ViewTarget, With<SceneCompositor>>,
    sources: Query<&ViewTarget, With<SceneSource>>,
    composite: Res<CompositePipeline>,
    cache: Res<PipelineCache>,
    // The scene's two main textures alternate as its post-processing ping-pongs.
    mut bind_groups: Local<Vec<(TextureViewId, BindGroup)>>,
    mut ctx: RenderContext,
) {
    let target = view.into_inner();
    let Some(source) = sources.iter().next() else {
        return;
    };
    let Some(pipeline) = composite
        .pipeline
        .and_then(|(_, id)| cache.get_render_pipeline(id))
    else {
        return;
    };
    let scene = source.main_texture_view();
    let index = match bind_groups.iter().position(|(id, _)| *id == scene.id()) {
        Some(index) => index,
        None => {
            if bind_groups.len() >= 2 {
                bind_groups.clear();
            }
            let bind_group = ctx.render_device().create_bind_group(
                "scene composite",
                &cache.get_bind_group_layout(&composite.layout),
                &BindGroupEntries::sequential((scene, &composite.sampler)),
            );
            bind_groups.push((scene.id(), bind_group));
            bind_groups.len() - 1
        }
    };
    // Taking the attachment consumes the target's pending clear, which this
    // full-screen write makes redundant.
    let mut attachment = target.get_unsampled_color_attachment();
    attachment.ops.load = LoadOp::Load;
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("scene composite"),
            color_attachments: &[Some(attachment)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &bind_groups[index].1, &[]);
    pass.draw(0..3, 0..1);
}
