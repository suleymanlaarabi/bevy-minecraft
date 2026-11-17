//! Native-resolution presentation in one pass: the scene is tonemapped straight
//! into the window and the interface is drawn over it in the same pass.
//!
//! Bevy tonemaps into the camera's second main texture, draws the interface there,
//! then copies that texture to the window: two more full-screen reads and writes
//! per frame, and a tonemapped image requantized to the scene's packed float format
//! before its 8-bit encoding. Nothing reads the tonemapped main texture when the
//! camera presents at native resolution, so those views skip both. With FXAA or
//! bloom, the same pass also anti-aliases and adds the bloom (`present_scene.wesl`,
//! `bloom.rs`): Bevy's final bloom blend, its separate tonemapping pass, its second
//! target and the copy all go.
//! Scaled rendering keeps its own single pass (`composite`).
use bevy::{
    anti_alias::fxaa::{Fxaa, Sensitivity, fxaa},
    camera::CameraOutputMode,
    core_pipeline::{
        Core3dSystems, FullscreenShader,
        schedule::Core3d,
        tonemapping::{
            DebandDither, Tonemapping, TonemappingLuts, get_lut_bind_group_layout_entries,
            get_lut_bindings, tonemapping,
        },
        upscaling::upscaling,
    },
    ecs::{entity::EntityHashMap, schedule::ScheduleCleanupPolicy},
    post_process::bloom::{Bloom, BloomCompositeMode, bloom},
    prelude::*,
    render::extract_component::{ExtractComponent, ExtractComponentPlugin},
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        render_asset::RenderAssets,
        render_phase::ViewSortedRenderPhases,
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{CurrentView, RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::{FallbackImage, GpuImage},
        view::{
            ExtractedView, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
            prepare_view_targets,
        },
    },
    shader::ShaderDefVal,
    ui_render::{TransparentUi, UiCameraView, ui_pass},
};

use super::bloom::{BloomPasses, render_pyramid};

/// Scene format, window format and tonemapping variant of a presentation pipeline.
type PresentKey = (TextureFormat, TextureFormat, Tonemapping, DebandDither);

/// FXAA's edge and dark thresholds.
type FxaaKey = (Sensitivity, Sensitivity);

/// Tonemapping, FXAA thresholds when anti-aliased, bloom blend when blooming.
type PipelineKey = (PresentKey, Option<FxaaKey>, Option<BloomCompositeMode>);

#[derive(Resource, Default)]
struct DirectPresentation {
    /// Bevy's tonemapping, upscaling and interface passes were made conditional.
    installed: bool,
    /// Bevy's FXAA and bloom passes were made conditional as well.
    fxaa: bool,
    bloom: bool,
    pipelines: Vec<(PipelineKey, CachedRenderPipelineId)>,
    /// The scene shader for FXAA or bloom, and its layouts without and with bloom:
    /// the tonemapping layout with a filtering sampler for bilinear taps.
    shader: Handle<Shader>,
    layouts: Option<[BindGroupLayoutDescriptor; 2]>,
    /// Views presenting directly this frame.
    views: EntityHashMap<Presented>,
}

#[derive(Clone, Copy)]
struct Presented {
    pipeline: CachedRenderPipelineId,
    /// Drawn by the scene shader rather than Bevy's tonemapping shader.
    scene_shader: bool,
    bloom: bool,
}

/// Opt in when no later pass needs the tonemapped main texture.
#[derive(Component, Clone, Copy, ExtractComponent)]
#[extract_app(RenderApp)]
pub struct DirectPresent;

pub(super) fn register(app: &mut App) {
    app.add_plugins(ExtractComponentPlugin::<DirectPresent>::default());
    let Some(render) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render
        .init_resource::<DirectPresentation>()
        // Once every plugin has added its passes.
        .add_systems(RenderStartup, install)
        .add_systems(
            Render,
            select_presentation
                .in_set(RenderSystems::PrepareViews)
                .after(prepare_view_targets),
        );
}

fn install(world: &mut World) {
    let shader = world
        .resource::<AssetServer>()
        .load("embedded://hollow_render/shaders/present_scene.wesl");
    let bloom_passes = BloomPasses::new(world);
    world.insert_resource(bloom_passes);
    {
        let mut direct = world.resource_mut::<DirectPresentation>();
        direct.shader = shader;
        let [lut, lut_sampler] = get_lut_bind_group_layout_entries();
        let entries = DynamicBindGroupLayoutEntries::new_with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, uniform_buffer::<ViewUniform>(true)),
                (1, texture_2d(TextureSampleType::Float { filterable: true })),
                (2, sampler(SamplerBindingType::Filtering)),
                (3, lut),
                (4, lut_sampler),
            ),
        );
        direct.layouts = Some([
            BindGroupLayoutDescriptor::new("present scene layout", &entries),
            BindGroupLayoutDescriptor::new(
                "present bloom scene layout",
                &entries.extend_with_indices((
                    (5, texture_2d(TextureSampleType::Float { filterable: true })),
                    (6, uniform_buffer::<Vec4>(false)),
                )),
            ),
        ]);
    }
    let mut installed = true;
    world.schedule_scope(Core3d, |world, schedule| {
        // The system sets stay, so the ordering other passes declare against these
        // three still holds for the conditional systems added back below.
        let policy = ScheduleCleanupPolicy::RemoveSystemsOnly;
        if matches!(
            schedule.remove_systems_in_set(tonemapping, world, policy),
            Ok(1)
        ) {
            schedule.add_systems(
                tonemapping
                    .in_set(Core3dSystems::PostProcess)
                    .run_if(not(presents_directly)),
            );
        } else {
            installed = false;
        }
        if matches!(
            schedule.remove_systems_in_set(ui_pass, world, policy),
            Ok(1)
        ) {
            schedule.add_systems(
                ui_pass
                    .after(Core3dSystems::PostProcess)
                    .before(upscaling)
                    .run_if(not(presents_directly)),
            );
        } else {
            installed = false;
        }
        if matches!(
            schedule.remove_systems_in_set(upscaling, world, policy),
            Ok(1)
        ) {
            schedule.add_systems(
                upscaling
                    .after(Core3dSystems::PostProcess)
                    .run_if(not(presents_directly)),
            );
        } else {
            installed = false;
        }
        let bloom_removed = matches!(
            schedule.remove_systems_in_set(bloom, world, policy),
            Ok(1)
        );
        if bloom_removed {
            schedule.add_systems(
                bloom
                    .before(tonemapping)
                    .in_set(Core3dSystems::PostProcess)
                    .run_if(not(presents_directly)),
            );
            schedule.add_systems(
                render_pyramid
                    .before(tonemapping)
                    .in_set(Core3dSystems::PostProcess)
                    .run_if(presents_directly),
            );
        }
        let fxaa_removed = matches!(
            schedule.remove_systems_in_set(fxaa, world, policy),
            Ok(1)
        );
        if fxaa_removed {
            schedule.add_systems(
                fxaa.after(tonemapping)
                    .in_set(Core3dSystems::PostProcess)
                    .run_if(not(presents_directly)),
            );
        }
        schedule.add_systems(
            present
                .after(Core3dSystems::PostProcess)
                .before(upscaling)
                .run_if(presents_directly),
        );
        let mut direct = world.resource_mut::<DirectPresentation>();
        direct.fxaa = fxaa_removed;
        direct.bloom = bloom_removed;
    });
    if !installed {
        error!("Bevy's presentation passes are kept: one of them was not found");
    }
    world.resource_mut::<DirectPresentation>().installed = installed;
}

fn presents_directly(view: Res<CurrentView>, direct: Res<DirectPresentation>) -> bool {
    direct.views.contains_key(&view.0)
}

/// Picks the views presented directly this frame. A view keeps Bevy's passes until
/// its pipelines are compiled, and whenever something still needs the tonemapped
/// main texture or shares the output with an earlier camera.
#[allow(clippy::too_many_arguments)]
fn select_presentation(
    mut direct: ResMut<DirectPresentation>,
    mut bloom_passes: ResMut<BloomPasses>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    fullscreen: Res<FullscreenShader>,
    cameras: Query<
        (
            Entity,
            &ExtractedCamera,
            &ExtractedView,
            &ViewTarget,
            &Tonemapping,
            Option<&DebandDither>,
            Option<&Fxaa>,
            Option<&Bloom>,
            Option<&UiCameraView>,
        ),
        With<DirectPresent>,
    >,
    mut ui_views: Query<&mut ExtractedView, Without<ExtractedCamera>>,
) {
    let direct = &mut *direct;
    direct.views.clear();
    if !direct.installed {
        bloom_passes.retain(|_| false);
        return;
    }
    for (entity, camera, view, target, tonemapping, dither, fxaa, bloom, ui) in &cameras {
        let Some(window_format) = target.out_texture_view_format() else {
            continue;
        };
        let fxaa = fxaa
            .filter(|fxaa| fxaa.enabled)
            .map(|fxaa| (fxaa.edge_threshold, fxaa.edge_threshold_min));
        let bloom = bloom.filter(|bloom| bloom.intensity > 0.0);
        if !camera.hdr
            || (fxaa.is_some() && !direct.fxaa)
            || (bloom.is_some() && !direct.bloom)
            || !tonemapping.is_enabled()
            || camera.viewport.is_some()
            || camera.sorted_camera_index_for_target != 0
            || matches!(camera.output_mode, CameraOutputMode::Skip)
            // The tonemapping shader writes linear values: the window encodes them.
            || !window_format.is_srgb()
        {
            continue;
        }
        let key = (
            (
                view.target_format,
                window_format,
                *tonemapping,
                dither.copied().unwrap_or(DebandDither::Disabled),
            ),
            fxaa,
            bloom.map(|bloom| bloom.composite_mode),
        );
        let scene_shader = fxaa.is_some() || bloom.is_some();
        let id = match direct.pipelines.iter().find(|(known, _)| *known == key) {
            Some(&(_, id)) => id,
            None => {
                // Bevy queues its own pipeline for the view during the first frame.
                let Some(mut descriptor) = tonemapping_descriptor(&cache, &key.0).cloned() else {
                    continue;
                };
                descriptor.label = Some("present scene".into());
                if let Some(fragment) = descriptor.fragment.as_mut() {
                    if scene_shader && let Some(layouts) = &direct.layouts {
                        descriptor.layout = vec![layouts[usize::from(bloom.is_some())].clone()];
                        fragment.shader = direct.shader.clone();
                        // The thresholds are declared even when FXAA is off.
                        let (edge, dark) = fxaa.unwrap_or((Sensitivity::High, Sensitivity::High));
                        fragment.shader_defs.extend([
                            format!("EDGE_THRESH_{}", edge.get_str()).into(),
                            format!("EDGE_THRESH_MIN_{}", dark.get_str()).into(),
                        ]);
                        if fxaa.is_some() {
                            fragment.shader_defs.push("FXAA".into());
                        }
                        if let Some(bloom) = bloom {
                            fragment.shader_defs.push("BLOOM".into());
                            if bloom.composite_mode == BloomCompositeMode::Additive {
                                fragment.shader_defs.push("BLOOM_ADDITIVE".into());
                            }
                        }
                    }
                    if let Some(Some(output)) = fragment.targets.first_mut() {
                        output.format = window_format;
                    }
                }
                let id = cache.queue_render_pipeline(descriptor);
                direct.pipelines.push((key, id));
                id
            }
        };
        // Bloom's own pipelines are queued on its first frame as well.
        let bloom_ready = match (bloom, camera.physical_viewport_size) {
            (None, _) => true,
            (Some(bloom), Some(size)) => bloom_passes.prepare(
                entity,
                size,
                bloom,
                &device,
                &queue,
                &cache,
                &fullscreen,
            ),
            (Some(_), None) => false,
        };
        if !bloom_ready || cache.get_render_pipeline(id).is_none() {
            continue;
        }
        direct.views.insert(
            entity,
            Presented {
                pipeline: id,
                scene_shader,
                bloom: bloom.is_some(),
            },
        );
        // The interface's pipelines are specialized for the texture it is drawn into.
        if let Some(mut ui_view) = ui.and_then(|ui| ui_views.get_mut(ui.0).ok()) {
            ui_view.target_format = window_format;
        }
    }
    bloom_passes.retain(|view| direct.views.get(&view).is_some_and(|presented| presented.bloom));
}

/// The pipeline Bevy's tonemapping pass uses for a view of this key. Its key and
/// layout are private, so the same shader, definitions and bindings are taken from
/// the descriptor it queued; only the output format differs for the window.
fn tonemapping_descriptor<'a>(
    cache: &'a PipelineCache,
    &(scene_format, _, tonemapping, dither): &PresentKey,
) -> Option<&'a RenderPipelineDescriptor> {
    let method = match tonemapping {
        Tonemapping::None | Tonemapping::Linear => "TONEMAP_METHOD_LINEAR",
        Tonemapping::Reinhard => "TONEMAP_METHOD_REINHARD",
        Tonemapping::ReinhardLuminance => "TONEMAP_METHOD_REINHARD_LUMINANCE",
        Tonemapping::AcesFitted => "TONEMAP_METHOD_ACES_FITTED",
        Tonemapping::AgX => "TONEMAP_METHOD_AGX",
        Tonemapping::SomewhatBoringDisplayTransform => {
            "TONEMAP_METHOD_SOMEWHAT_BORING_DISPLAY_TRANSFORM"
        }
        Tonemapping::TonyMcMapface => "TONEMAP_METHOD_TONY_MC_MAPFACE",
        Tonemapping::BlenderFilmic => "TONEMAP_METHOD_BLENDER_FILMIC",
        Tonemapping::KhronosPbrNeutral => "TONEMAP_METHOD_PBR_NEUTRAL",
    };
    let dithered = matches!(dither, DebandDither::Enabled);
    cache.pipelines().find_map(|pipeline| {
        let PipelineDescriptor::RenderPipelineDescriptor(descriptor) = &pipeline.descriptor else {
            return None;
        };
        let fragment = descriptor.fragment.as_ref()?;
        let defined = |name: &str| {
            fragment
                .shader_defs
                .iter()
                .any(|def| matches!(def, ShaderDefVal::Bool(def, true) if def == name))
        };
        (descriptor.label.as_deref() == Some("tonemapping pipeline")
            && matches!(fragment.targets.first(), Some(Some(output)) if output.format == scene_format)
            && defined(method)
            && defined("DEBAND_DITHER") == dithered)
            .then_some(&**descriptor)
    })
}

/// Bindings of the present shaders: view uniforms, scene, tonemapping LUT, bloom.
type Bound = (
    BufferId,
    TextureViewId,
    TextureViewId,
    Option<TextureViewId>,
    CachedRenderPipelineId,
);

#[allow(clippy::too_many_arguments)]
fn present(
    world: &World,
    view: ViewQuery<
        (
            &ViewUniformOffset,
            &ViewTarget,
            &Tonemapping,
            Option<&UiCameraView>,
        ),
        With<DirectPresent>,
    >,
    ui_views: Query<&ExtractedView>,
    ui_phases: Res<ViewSortedRenderPhases<TransparentUi>>,
    direct: Res<DirectPresentation>,
    bloom_passes: Res<BloomPasses>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    uniforms: Res<ViewUniforms>,
    luts: Res<TonemappingLuts>,
    mut samplers: Local<Option<(Sampler, Sampler)>>,
    mut bound: Local<Option<(Bound, BindGroup)>>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let (uniform_offset, target, tonemapping, ui) = view.into_inner();
    let Some(&presented) = direct.views.get(&entity) else {
        return;
    };
    let id = presented.pipeline;
    let pyramid = bloom_passes.pyramid(entity);
    if presented.bloom && pyramid.is_none() {
        return;
    }
    let (Some(pipeline), Some(uniform_buffer)) =
        (cache.get_render_pipeline(id), uniforms.uniforms.buffer())
    else {
        return;
    };
    // The scene stays in the current main texture: nothing is written back to it.
    let scene = target.main_texture_view();
    let (lut, lut_sampler) = get_lut_bindings(&images, &luts, tonemapping, &fallback);
    let bloom = pyramid
        .filter(|_| presented.bloom)
        .map(|pyramid| (pyramid.first_level(), pyramid.composite()));
    let key = (
        uniform_buffer.id(),
        scene.id(),
        lut.id(),
        bloom.map(|(level, _)| level.id()),
        id,
    );
    if bound.as_ref().is_none_or(|(current, _)| *current != key) {
        let device = ctx.render_device();
        let (nearest, linear) = samplers.get_or_insert_with(|| {
            (
                device.create_sampler(&SamplerDescriptor::default()),
                // Bevy's FXAA sampler, for bilinear FXAA and bloom taps.
                device.create_sampler(&SamplerDescriptor {
                    mipmap_filter: MipmapFilterMode::Linear,
                    mag_filter: FilterMode::Linear,
                    min_filter: FilterMode::Linear,
                    ..default()
                }),
            )
        });
        let layout =
            cache.get_bind_group_layout(&cache.get_render_pipeline_descriptor(id).layout[0]);
        let sampler = if presented.scene_shader {
            &*linear
        } else {
            &*nearest
        };
        let bind_group = match bloom {
            Some((level, composite)) => device.create_bind_group(
                "present scene",
                &layout,
                &BindGroupEntries::sequential((
                    &uniforms.uniforms,
                    scene,
                    sampler,
                    lut,
                    lut_sampler,
                    level,
                    composite.as_entire_binding(),
                )),
            ),
            None => device.create_bind_group(
                "present scene",
                &layout,
                &BindGroupEntries::sequential((
                    &uniforms.uniforms,
                    scene,
                    sampler,
                    lut,
                    lut_sampler,
                )),
            ),
        };
        *bound = Some((key, bind_group));
    }
    let Some((_, bind_group)) = bound.as_ref() else {
        return;
    };
    // Every pixel is written: the window's previous contents are dropped.
    let Some(attachment) = target.out_texture_color_attachment(Some(LinearRgba::BLACK)) else {
        return;
    };
    let ui = ui.and_then(|ui| {
        let phase = ui_phases.get(&ui_views.get(ui.0).ok()?.retained_view_entity)?;
        (!phase.items.is_empty()).then_some((ui.0, phase))
    });

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("present"),
        color_attachments: &[Some(attachment)],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let span = diagnostics.pass_span(&mut pass, "present");
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[uniform_offset.offset]);
    pass.draw(0..3, 0..1);
    if let Some((ui_view, phase)) = ui
        && let Err(err) = phase.render(&mut pass, world, ui_view)
    {
        error!("Error encountered while rendering the ui phase {err:?}");
    }
    span.end(&mut pass);
}
