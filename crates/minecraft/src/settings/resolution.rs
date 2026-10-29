use bevy::{
    camera::{CameraUpdateSystems, RenderTarget, visibility::RenderLayers},
    image::ImageSampler,
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureFormat},
        renderer::RenderDevice,
    },
    window::PrimaryWindow,
};

use super::GraphicsSettings;
use crate::player::PlayerCamera;

#[derive(Component)]
struct RenderSurface {
    image: Handle<Image>,
    compositor: Entity,
    display: Entity,
}

pub(super) fn register(app: &mut App) {
    app.add_systems(PostUpdate, sync_resolution.before(CameraUpdateSystems));
}

fn sync_resolution(
    settings: Res<GraphicsSettings>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(Entity, &mut RenderTarget, Option<&RenderSurface>), With<PlayerCamera>>,
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
    for (entity, mut target, surface) in &mut cameras {
        if size == window_size {
            if let Some(surface) = surface {
                *target = RenderTarget::default();
                commands.entity(surface.compositor).despawn();
                commands.entity(surface.display).despawn();
                commands
                    .entity(entity)
                    .remove::<RenderSurface>()
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
        let mut image =
            Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
        image.sampler = ImageSampler::linear();
        let image = images.add(image);
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
                ChildOf(entity),
            ))
            .id();
        let display = commands
            .spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                ImageNode::new(image.clone()),
                UiTargetCamera(compositor),
                GlobalZIndex(-2),
                Pickable::IGNORE,
                DespawnOnExit(crate::game::GameState::Game),
            ))
            .id();
        *target = RenderTarget::Image(image.clone().into());
        commands
            .entity(entity)
            .remove::<IsDefaultUiCamera>()
            .insert(RenderSurface {
                image,
                compositor,
                display,
            });
    }
}
