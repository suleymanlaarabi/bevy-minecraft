use bevy::{
    anti_alias::fxaa::Fxaa,
    core_pipeline::prepass::DepthPrepass,
    light::DirectionalLightShadowMap,
    prelude::*,
    render::occlusion_culling::OcclusionCulling,
    window::{MonitorSelection, PresentMode, WindowMode},
};

use super::{AntiAliasingMode, DisplayMode, GraphicsSettings, ShadowQuality};
use crate::player::PlayerCamera;

const WINDOWED_WIDTH: f32 = 1600.0;
const WINDOWED_HEIGHT: f32 = 900.0;

pub(super) fn register(app: &mut App) {
    app.add_observer(apply_graphics_settings)
        .add_observer(apply_settings_to_new_target);
}

fn apply_graphics_settings(
    _insert: On<Insert<GraphicsSettings>>,
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window>,
    mut cameras: Query<(Entity, &mut Projection), With<PlayerCamera>>,
    mut lights: Query<(Entity, &mut DirectionalLight)>,
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
    for (entity, mut light) in &mut lights {
        apply_light_settings(&settings, entity, &mut light, &mut commands);
    }
    if let (Some(size), Some(mut shadow_map)) = (settings.shadow_quality.map_size(), shadow_map) {
        shadow_map.size = size;
    }
}

fn apply_settings_to_new_target(
    add: On<Add<(Window, PlayerCamera, DirectionalLight)>>,
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window>,
    mut cameras: Query<&mut Projection, With<PlayerCamera>>,
    mut lights: Query<&mut DirectionalLight>,
    mut commands: Commands,
) {
    let settings = settings.normalized();
    if let Ok(mut window) = windows.get_mut(add.entity) {
        apply_window_settings(&settings, &mut window);
    }
    if let Ok(mut projection) = cameras.get_mut(add.entity) {
        apply_camera_settings(&settings, add.entity, &mut projection, &mut commands);
    }
    if let Ok(mut light) = lights.get_mut(add.entity) {
        apply_light_settings(&settings, add.entity, &mut light, &mut commands);
    }
}

fn apply_light_settings(
    settings: &GraphicsSettings,
    entity: Entity,
    light: &mut DirectionalLight,
    commands: &mut Commands,
) {
    light.shadow_maps_enabled = settings.shadow_quality != ShadowQuality::Off;
    // Also culls shadow casters hidden from the sun in every cascade.
    let mut light = commands.entity(entity);
    if settings.occlusion_culling {
        light.insert(OcclusionCulling);
    } else {
        light.remove::<OcclusionCulling>();
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
    // Depth first: the costly terrain shading then runs once per pixel, and the
    // depth pyramid lets occlusion culling skip whole sections hidden by terrain.
    camera.insert(DepthPrepass);
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
}
