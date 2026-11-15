use bevy::{
    anti_alias::fxaa::Fxaa,
    core_pipeline::prepass::DepthPrepass,
    light::DirectionalLightShadowMap,
    post_process::bloom::Bloom,
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
    camera.insert((
        settings.anti_aliasing.msaa(),
        bevy_voxel::render::presentation::SceneResolution(settings.render_resolution.height()),
        bevy_voxel::render::presentation::DirectPresent,
        bevy_voxel::render::presentation::CompactHdr,
        bevy_voxel::render::presentation::UnsampledDepthPrepass,
    ));
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
