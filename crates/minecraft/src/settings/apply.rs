use bevy::{
    anti_alias::fxaa::Fxaa,
    core_pipeline::{core_3d::extract_camera_prepass_phase, prepass::DepthPrepass},
    light::DirectionalLightShadowMap,
    post_process::bloom::Bloom,
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp,
        camera::{CameraMainPassTextureFormats, extract_cameras},
        render_resource::{TextureFormat, TextureFormatFeatureFlags, TextureUsages, WgpuFeatures},
        renderer::{RenderAdapter, RenderDevice},
        sync_world::RenderEntity,
        view::ExtractedView,
    },
    ui_render::RenderUiSystems,
    window::{MonitorSelection, PresentMode, WindowMode},
};

use super::{AntiAliasingMode, DisplayMode, GraphicsSettings, ShadowQuality};
use crate::player::PlayerCamera;

const WINDOWED_WIDTH: f32 = 1600.0;
const WINDOWED_HEIGHT: f32 = 900.0;

pub(super) fn register(app: &mut App) {
    app.add_observer(apply_graphics_settings)
        .add_observer(apply_settings_to_new_target);
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.add_systems(
            ExtractSchedule,
            (
                skip_prepass_depth_copy.after(extract_camera_prepass_phase),
                // The interface's views copy the camera's format when they are extracted.
                compact_scene_target
                    .after(extract_cameras)
                    .before(RenderUiSystems::ExtractCameraViews),
            ),
        );
    }
}

/// Bevy copies the depth prepass into a separate sampled texture after the prepass,
/// for materials reading `depth_prepass_texture`. Nothing samples it here: no Hollow
/// shader does, and no Bevy feature that does (transmission, SSR, SSAO, decals, OIT,
/// TAA) is used. The prepass phases and early depth only need the main-world
/// `DepthPrepass`, so the render-world marker is dropped to skip the copy.
fn skip_prepass_depth_copy(
    cameras: Extract<Query<RenderEntity, (With<PlayerCamera>, With<DepthPrepass>)>>,
    mut commands: Commands,
) {
    for entity in &cameras {
        commands.entity(entity).remove::<DepthPrepass>();
    }
}

/// Bevy's bloom already blurs the scene in this packed 32-bit float format.
const COMPACT_HDR: TextureFormat = TextureFormat::Rg11b10Ufloat;

/// Bevy gives an HDR camera 64-bit `Rgba16Float` color targets. Every scene pass
/// reads or writes them: the multisampled color, its resolve, bloom, tonemapping
/// and the final copy, which is most of the memory traffic of a frame on an
/// integrated GPU. The scene needs no alpha, and its smooth gradients are dithered
/// against the format's 6-bit mantissa (`hdr_dither` in `sky_color.wesl`), so the
/// packed format carries the same image with half the bytes.
fn compact_scene_target(
    cameras: Extract<Query<(RenderEntity, &Msaa), (With<PlayerCamera>, With<bevy::camera::Hdr>)>>,
    device: Res<RenderDevice>,
    adapter: Res<RenderAdapter>,
    mut formats: ResMut<CameraMainPassTextureFormats>,
    mut supported: Local<Option<(u32, bool)>>,
    mut commands: Commands,
) {
    for (entity, msaa) in &cameras {
        let samples = msaa.samples();
        let compact = match *supported {
            Some((checked, compact)) if checked == samples => compact,
            _ => {
                let compact = compact_hdr_supported(&device, &adapter, samples);
                *supported = Some((samples, compact));
                compact
            }
        };
        let Some(format) = formats.get_mut(&entity).filter(|_| compact) else {
            continue;
        };
        *format = COMPACT_HDR;
        // `extract_cameras` inserts this frame's view with a command of its own.
        commands.entity(entity).queue(|mut camera: EntityWorldMut| {
            if let Some(mut view) = camera.get_mut::<ExtractedView>() {
                view.target_format = COMPACT_HDR;
            }
        });
    }
}

/// Rendering, blending and resolving into the packed format are optional features.
fn compact_hdr_supported(device: &RenderDevice, adapter: &RenderAdapter, samples: u32) -> bool {
    let features = device.features();
    if !features.contains(WgpuFeatures::RG11B10UFLOAT_RENDERABLE) {
        return false;
    }
    let format = if features.contains(WgpuFeatures::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
        adapter.get_texture_format_features(COMPACT_HDR)
    } else {
        COMPACT_HDR.guaranteed_format_features(features)
    };
    format
        .allowed_usages
        .contains(TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING)
        && format.flags.contains(TextureFormatFeatureFlags::BLENDABLE)
        && (samples == 1
            || format.flags.sample_count_supported(samples)
                && format
                    .flags
                    .contains(TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
}

fn apply_graphics_settings(
    _insert: On<Insert<GraphicsSettings>>,
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window>,
    mut cameras: Query<(Entity, &mut Projection), With<PlayerCamera>>,
    shadow_map: Option<ResMut<DirectionalLightShadowMap>>,
    mut commands: Commands,
) {
    let settings = settings.normalized();
    commands.insert_resource_if_neq(settings);

    for window in windows
        .contiguous_iter_mut()
        .expect("Dense window query")
        .flatten()
    {
        apply_window_settings(&settings, window);
    }
    for (entity, projection) in cameras
        .contiguous_iter_mut()
        .expect("Dense player projection query")
        .flat_map(|(entities, projections)| entities.iter().copied().zip(projections))
    {
        apply_camera_settings(&settings, entity, projection, &mut commands);
    }
    if let (Some(size), Some(mut shadow_map)) = (settings.shadow_quality.map_size(), shadow_map) {
        shadow_map.size = size;
    }
}

fn apply_settings_to_new_target(
    add: On<Add<(Window, PlayerCamera)>>,
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window>,
    mut cameras: Query<&mut Projection, With<PlayerCamera>>,
    mut commands: Commands,
) {
    let settings = settings.normalized();
    if let Ok(mut window) = windows.get_mut(add.entity) {
        apply_window_settings(&settings, &mut window);
    }
    if let Ok(mut projection) = cameras.get_mut(add.entity) {
        apply_camera_settings(&settings, add.entity, &mut projection, &mut commands);
    }
}

fn apply_window_settings(settings: &GraphicsSettings, window: &mut Window) {
    window.present_mode = if settings.vsync {
        PresentMode::Fifo
    } else {
        PresentMode::AutoNoVsync
    };

    match settings.display_mode {
        DisplayMode::Windowed => {
            if window.mode != WindowMode::Windowed {
                window.mode = WindowMode::Windowed;
                window.resolution.set(WINDOWED_WIDTH, WINDOWED_HEIGHT);
            }
        }
        DisplayMode::BorderlessFullscreen => {
            window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Primary);
        }
    }
}

fn apply_camera_settings(
    settings: &GraphicsSettings,
    entity: Entity,
    projection: &mut Projection,
    commands: &mut Commands,
) {
    if let Projection::Perspective(perspective) = projection {
        perspective.fov = settings.field_of_view.to_radians();
    }

    let mut camera = commands.entity(entity);
    camera.insert(settings.anti_aliasing.msaa());
    // Depth first: the costly shadowed terrain shading then runs once per pixel.
    // Without shadows that shading is a texture, vertex light and fog, and sections
    // already draw front to back: a second pass over the near terrain's vertices costs
    // more than the few overdrawn fragments, so the main pass writes depth itself.
    // Bevy's GPU occlusion culling stays off: terrain sections are culled on the CPU
    // by the section visibility graph and never by Bevy, so it would downsample the
    // whole multisampled depth buffer into a pyramid and run a second prepass every
    // frame for the few Bevy meshes left.
    if settings.shadow_quality != ShadowQuality::Off {
        camera.insert(DepthPrepass);
    } else {
        camera.remove::<DepthPrepass>();
    }
    if settings.anti_aliasing == AntiAliasingMode::Fxaa {
        camera.insert(Fxaa::default());
    } else {
        camera.remove::<Fxaa>();
    }
    if settings.bloom {
        // Subtle: a halo around the sun and moon, not a haze over the terrain.
        camera.insert(Bloom {
            intensity: 0.08,
            ..Bloom::NATURAL
        });
    } else {
        camera.remove::<Bloom>();
    }
}
