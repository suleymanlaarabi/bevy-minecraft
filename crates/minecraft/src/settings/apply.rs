use bevy::{
    anti_alias::fxaa::Fxaa,
    core_pipeline::{core_3d::extract_camera_prepass_phase, prepass::DepthPrepass},
    light::DirectionalLightShadowMap,
    post_process::bloom::Bloom,
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp, occlusion_culling::OcclusionCulling,
        sync_world::RenderEntity,
    },
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
            skip_prepass_depth_copy.after(extract_camera_prepass_phase),
        );
    }
}

/// Bevy copies the depth prepass into a separate sampled texture after each prepass
/// (twice per frame with occlusion culling), for materials reading
/// `depth_prepass_texture`. Nothing samples it here: no Hollow shader does, and no
/// Bevy feature that does (transmission, SSR, SSAO, decals, OIT, TAA) is used.
/// The prepass phases, early depth and occlusion culling only need the main-world
/// `DepthPrepass`, so the render-world marker is dropped to skip the copies.
fn skip_prepass_depth_copy(
    cameras: Extract<Query<RenderEntity, (With<PlayerCamera>, With<DepthPrepass>)>>,
    mut commands: Commands,
) {
    for entity in &cameras {
        commands.entity(entity).remove::<DepthPrepass>();
    }
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

    for mut window in &mut windows {
        apply_window_settings(&settings, &mut window);
    }
    for (entity, mut projection) in &mut cameras {
        apply_camera_settings(&settings, entity, &mut projection, &mut commands);
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
    // Occlusion culling reads the prepass depth and stays on the camera only: in
    // shadow cascades it would build four depth pyramids per frame for the few Bevy
    // meshes there, never for terrain sections.
    if settings.occlusion_culling || settings.shadow_quality != ShadowQuality::Off {
        camera.insert(DepthPrepass);
    } else {
        camera.remove::<DepthPrepass>();
    }
    if settings.occlusion_culling {
        camera.insert(OcclusionCulling);
    } else {
        camera.remove::<OcclusionCulling>();
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
