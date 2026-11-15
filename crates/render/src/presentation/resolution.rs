use bevy::{
    camera::{CameraOutputMode, CameraUpdateSystems, RenderTarget, visibility::RenderLayers},
    prelude::*,
    render::renderer::RenderDevice,
    window::{PrimaryWindow, WindowResized},
};

use super::composite::{SceneCompositor, SceneSource};

/// Opt a scene camera into aspect-preserving presentation at the requested height.
#[derive(Component, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneResolution(pub Option<u32>);
impl SceneResolution {
    pub fn size(self, window: UVec2, max_dimension: u32) -> UVec2 {
        let window = window.max(UVec2::ONE);
        let Some(height) = self.0 else {
            return window;
        };
        let height = height.max(1).min(max_dimension.max(1));
        let width = (u64::from(window.x) * u64::from(height) + u64::from(window.y) / 2)
            / u64::from(window.y);
        let limit = u64::from(max_dimension.max(1));
        if width > limit {
            UVec2::new(
                limit as u32,
                (u64::from(height) * limit / width).max(1) as u32,
            )
        } else {
            UVec2::new(width.max(1) as u32, height)
        }
    }
}

#[derive(Component)]
struct RenderSurface {
    compositor: Entity,
}

pub(super) fn register(app: &mut App) {
    // Only settings, window sizes and new cameras change the target resolution.
    app.add_systems(
        PostUpdate,
        sync_resolution.before(CameraUpdateSystems).run_if(
            on_message::<WindowResized>.or_else(any_match_filter::<Changed<SceneResolution>>),
        ),
    );
}

fn sync_resolution(
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(
        Entity,
        &SceneResolution,
        &mut Camera,
        &mut RenderTarget,
        Option<&RenderSurface>,
    )>,
    device: Res<RenderDevice>,
    mut commands: Commands,
) {
    let window_size = window.physical_size();
    if window_size.x == 0 || window_size.y == 0 {
        return;
    }
    for (entity, resolution, mut camera, mut target, surface) in &mut cameras {
        let size = resolution.size(window_size, device.limits().max_texture_dimension_2d);
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
        if surface.is_some() {
            if !matches!(*target, RenderTarget::None { size: current } if current == size) {
                *target = RenderTarget::None { size };
            }
            continue;
        }
        // Explicit offscreen targets (such as headless diagnostics) own their resolution.
        if !matches!(*target, RenderTarget::Window(_)) {
            continue;
        }
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
        *target = RenderTarget::None { size };
        camera.output_mode = CameraOutputMode::Skip;
        commands
            .entity(entity)
            .remove::<IsDefaultUiCamera>()
            .insert((RenderSurface { compositor }, SceneSource));
    }
}
