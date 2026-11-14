use bevy::{
    camera::{CameraOutputMode, CameraUpdateSystems, RenderTarget, visibility::RenderLayers},
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureFormat},
        renderer::RenderDevice,
    },
    window::{PrimaryWindow, WindowResized},
};

use super::{
    GraphicsSettings,
    composite::{SceneCompositor, SceneSource},
};
use crate::player::PlayerCamera;

#[derive(Component)]
struct RenderSurface {
    image: Handle<Image>,
    compositor: Entity,
}

pub(super) fn register(app: &mut App) {
    // Only settings, window sizes and new cameras change the target resolution.
    app.add_systems(
        PostUpdate,
        sync_resolution.before(CameraUpdateSystems).run_if(
            resource_changed::<GraphicsSettings>
                .or_else(on_message::<WindowResized>)
                .or_else(any_match_filter::<Added<PlayerCamera>>),
        ),
    );
}

fn sync_resolution(
    settings: Res<GraphicsSettings>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<
        (
            Entity,
            &mut Camera,
            &mut RenderTarget,
            Option<&RenderSurface>,
        ),
        With<PlayerCamera>,
    >,
    mut images: ResMut<Assets<Image>>,
    device: Res<RenderDevice>,
    mut commands: Commands,
) {
    let window_size = window.physical_size();
    if window_size.x == 0 || window_size.y == 0 {
        return;
    }
    let size = settings
        .render_resolution
        .size(window_size, device.limits().max_texture_dimension_2d);
    for (entity, mut camera, mut target, surface) in &mut cameras {
        if size == window_size {
            if let Some(surface) = surface {
                *target = RenderTarget::default();
                camera.output_mode = CameraOutputMode::default();
                commands.entity(surface.compositor).despawn();
                commands
                    .entity(entity)
                    .remove::<(RenderSurface, SceneSource)>()
                    .insert(IsDefaultUiCamera);
            }
            continue;
        }
        if let Some(surface) = surface {
            if images
                .get(&surface.image)
                .is_some_and(|image| image.size() != size)
                && let Some(mut image) = images.get_mut(&surface.image)
            {
                image.resize(Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                });
            }
            continue;
        }
        // Explicit offscreen targets (such as headless diagnostics) own their resolution.
        if !matches!(*target, RenderTarget::Window(_)) {
            continue;
        }
        // The image only gives the scene camera its resolution: the compositor
        // samples the camera's main texture itself (see `composite`), so the
        // camera skips writing its output here.
        let image = images.add(Image::new_target_texture(
            size.x,
            size.y,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        // Only scaled rendering needs a second camera. HUD and hand overlays remain
        // in window coordinates; native rendering keeps its existing direct path.
        let compositor = commands
            .spawn((
                Camera2d,
                Camera {
                    order: 1,
                    ..default()
                },
                Msaa::Off,
                RenderLayers::none(),
                IsDefaultUiCamera,
                SceneCompositor,
                ChildOf(entity),
            ))
            .id();
        *target = RenderTarget::Image(image.clone().into());
        camera.output_mode = CameraOutputMode::Skip;
        commands
            .entity(entity)
            .remove::<IsDefaultUiCamera>()
            .insert((RenderSurface { image, compositor }, SceneSource));
    }
}
