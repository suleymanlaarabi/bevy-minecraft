use bevy::{
    anti_alias::fxaa::Fxaa,
    light::DirectionalLightShadowMap,
    prelude::*,
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
    _insert: On<Insert, GraphicsSettings>,
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window>,
    mut cameras: Query<(Entity, &mut Projection), With<PlayerCamera>>,
    mut lights: Query<&mut DirectionalLight>,
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
    for mut light in &mut lights {
        light.shadow_maps_enabled = settings.shadow_quality != ShadowQuality::Off;
    }
    if let (Some(size), Some(mut shadow_map)) = (settings.shadow_quality.map_size(), shadow_map) {
        shadow_map.size = size;
    }
}

fn apply_settings_to_new_target(
    add: On<Add, (Window, PlayerCamera, DirectionalLight)>,
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
        light.shadow_maps_enabled = settings.shadow_quality != ShadowQuality::Off;
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
    if settings.anti_aliasing == AntiAliasingMode::Fxaa {
        camera.insert(Fxaa::default());
    } else {
        camera.remove::<Fxaa>();
    }
}
