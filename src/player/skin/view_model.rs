//! Camera-local hand posing and attack cycles, independent of block destruction time.
use super::*;
use crate::player::PlayerCamera;
use bevy::{camera::CameraProjection, render::render_resource::Extent3d, window::PrimaryWindow};
use std::{f32::consts::PI, sync::OnceLock};

const SWING_DURATION: f32 = 0.3;

#[derive(Component, Default)]
pub(crate) struct ArmSwing {
    elapsed: Option<f32>,
    requested: bool,
}

impl ArmSwing {
    pub(crate) fn request(&mut self) {
        self.requested = true;
    }

    fn progress(&self) -> f32 {
        self.elapsed.map_or(0., |elapsed| elapsed / SWING_DURATION)
    }
}

pub(super) fn cancel_swing(mut swings: Query<&mut ArmSwing>) {
    for mut swing in &mut swings {
        *swing = ArmSwing::default();
    }
}

pub(super) fn advance_swing(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<State<InventoryState>>,
    mut swing: Single<&mut ArmSwing, With<PlayerCamera>>,
) {
    if *state.get() != InventoryState::Closed {
        return;
    }
    let clicked = mouse.just_pressed(MouseButton::Left);
    if swing.elapsed.is_none() && !swing.requested && !clicked {
        return;
    }
    if let Some(elapsed) = &mut swing.elapsed {
        *elapsed += time.delta_secs();
        if *elapsed >= SWING_DURATION {
            swing.elapsed = None;
        }
    }
    // A click swings even in air. Mining requests each frame, repeating complete
    // cycles while the button is held without restarting the pose every frame.
    if swing.elapsed.is_none() && (swing.requested || clicked) {
        swing.elapsed = Some(0.);
    }
    swing.requested = false;
}

pub(super) fn first_person_pose(progress: f32, slim: bool) -> Transform {
    let strike = (progress.sqrt() * PI).sin();
    let twist = (progress * progress * PI).sin();
    // Minecraft 26.3 FirstPersonHandsAndItemsRenderer.renderPlayerArm, right hand,
    // fully equipped. Preserve the transform order rather than approximating Euler angles.
    // Cache the constant part, including AvatarRenderer.renderHand's shoulder pose.
    static BASES: OnceLock<[Mat4; 2]> = OnceLock::new();
    let bases = BASES.get_or_init(|| {
        let hand = Mat4::from_translation(Vec3::new(-1., 3.6, 3.5))
            * Mat4::from_rotation_z(120_f32.to_radians())
            * Mat4::from_rotation_x(200_f32.to_radians())
            * Mat4::from_rotation_y(-135_f32.to_radians())
            * Mat4::from_translation(Vec3::new(5.6, 0., 0.));
        std::array::from_fn(|model| {
            // Native cubes use Y-down, front -Z, 1/16 units and a shoulder at (-5,2,0).
            // Our shared skin meshes use Y-up, front +Z and centered arm widths.
            let center_x = if model == 0 { -1. } else { -0.5 };
            hand * Mat4::from_translation(Vec3::new(-5., 2., 0.) / 16.)
                * Mat4::from_rotation_z(0.1)
                * Mat4::from_translation(Vec3::new(center_x, -2., 0.) / 16.)
                * Mat4::from_rotation_x(PI)
                * Mat4::from_scale(Vec3::splat(1. / (16. * mesh::PIXEL)))
        })
    });
    let motion = Mat4::from_translation(Vec3::new(
        0.64 - 0.3 * strike,
        -0.6 + 0.4 * (progress.sqrt() * PI * 2.).sin(),
        -0.72 - 0.4 * (progress * PI).sin(),
    )) * Mat4::from_rotation_y((45. + 70. * strike).to_radians())
        * Mat4::from_rotation_z((-20. * twist).to_radians());
    Transform::from_matrix(motion * bases[usize::from(slim)])
}

/// Render only the arm's physical pixel rectangle, with the full-view projection.
/// All transforms are camera-local: camera movement cancels out of this calculation.
pub(super) fn resize_arm_target(
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<SkinAssets>,
    meshes: Res<Assets<Mesh>>,
    ui_scale: Res<UiScale>,
    arms: Query<(Entity, &Transform), With<FirstPersonArm>>,
    parts: Query<(&Mesh3d, &Transform, &ChildOf), With<SkinPart>>,
    mut cameras: Query<(&mut Camera, &Projection), With<ArmCamera>>,
    mut overlays: Query<(&mut Node, &mut Visibility), With<ArmView>>,
    mut images: ResMut<Assets<Image>>,
    mut allocation: Local<(UVec2, UVec2)>,
) {
    let full_size = UVec2::new(
        window.physical_width().max(1),
        window.physical_height().max(1),
    );
    for (mut camera, projection) in &mut cameras {
        if !camera.is_active {
            continue;
        }
        let mut min = full_size.as_vec2();
        let mut max = Vec2::ZERO;
        let mut bounded = false;
        let mut fallback = false;
        if let Projection::Perspective(perspective) = projection {
            let mut perspective = perspective.clone();
            perspective.aspect_ratio = full_size.x as f32 / full_size.y as f32;
            let clip_from_view = perspective.get_clip_from_view();
            for (root, arm) in &arms {
                for (mesh, part, parent) in &parts {
                    if parent.parent() != root {
                        continue;
                    }
                    let Some(bounds) = meshes.get(&mesh.0).and_then(|mesh| mesh.final_aabb) else {
                        fallback = true;
                        continue;
                    };
                    let clip_from_local = clip_from_view * arm.to_matrix() * part.to_matrix();
                    let low = bounds.min;
                    let high = bounds.max;
                    for corner in 0..8 {
                        let position = Vec3::new(
                            if corner & 1 == 0 { low.x } else { high.x },
                            if corner & 2 == 0 { low.y } else { high.y },
                            if corner & 4 == 0 { low.z } else { high.z },
                        );
                        let clip = clip_from_local * position.extend(1.0);
                        if !clip.is_finite() || clip.w <= perspective.near {
                            fallback = true;
                            continue;
                        }
                        let ndc = clip.truncate() / clip.w;
                        let pixel = (ndc.xy() * Vec2::new(0.5, -0.5) + Vec2::splat(0.5))
                            * full_size.as_vec2();
                        min = min.min(pixel);
                        max = max.max(pixel);
                        bounded = true;
                    }
                }
            }
        } else {
            fallback = true;
        }
        let (mut offset, mut size) = if fallback || !bounded {
            (Vec2::ZERO, full_size)
        } else {
            let low = (((min - Vec2::splat(8.0)) / 64.0).floor() * 64.0)
                .clamp(Vec2::ZERO, full_size.as_vec2());
            let high = (((max + Vec2::splat(8.0)) / 64.0).ceil() * 64.0)
                .clamp(Vec2::ZERO, full_size.as_vec2());
            (low, (high - low).max(Vec2::ZERO).as_uvec2())
        };
        if size.x == 0 || size.y == 0 {
            camera.is_active = false;
            for (_, mut visibility) in &mut overlays {
                visibility.set_if_neq(Visibility::Hidden);
            }
            continue;
        }
        if !fallback && bounded {
            if allocation.0 != full_size {
                *allocation = (full_size, UVec2::ZERO);
            }
            // Keep a high-water allocation through swings. Moving the crop is
            // cheap; recreating its color/depth textures every 64 pixels is not.
            allocation.1 = allocation.1.max(size).min(full_size);
            size = allocation.1;
            offset = offset.min((full_size - size).as_vec2());
        }
        let sub_view = bevy::camera::SubCameraView {
            full_size,
            offset,
            size,
        };
        if camera.sub_camera_view != Some(sub_view) {
            camera.sub_camera_view = Some(sub_view);
        }
        if images
            .get(&assets.arm_view)
            .is_some_and(|image| image.size() != size)
            && let Some(mut image) = images.get_mut(&assets.arm_view)
        {
            image.resize(Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            });
        }
        let scale = window.scale_factor() * ui_scale.0;
        for (mut node, _) in &mut overlays {
            let (left, top) = (px(offset.x / scale), px(offset.y / scale));
            let (width, height) = (px(size.x as f32 / scale), px(size.y as f32 / scale));
            if node.left != left || node.top != top || node.width != width || node.height != height
            {
                node.left = left;
                node.top = top;
                node.width = width;
                node.height = height;
            }
        }
    }
}

pub(super) fn animate_first_person(
    state: Res<State<InventoryState>>,
    view: Res<PlayerView>,
    settings: Res<SkinSettings>,
    swing: Single<Ref<ArmSwing>, With<PlayerCamera>>,
    mut arms: Query<(&mut Transform, &mut Visibility), (With<FirstPersonArm>, Without<ArmView>)>,
    mut overlays: Query<&mut Visibility, (With<ArmView>, Without<FirstPersonArm>)>,
    mut cameras: Query<&mut Camera, With<ArmCamera>>,
) {
    // Mesh rig roots and UI image roots are genuinely disjoint entity sets.
    let visible = *state.get() == InventoryState::Closed && *view == PlayerView::FirstPerson;
    for (mut transform, mut visibility) in &mut arms {
        visibility.set_if_neq(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if visible
            && (swing.is_changed()
                || settings.is_changed()
                || view.is_changed()
                || transform.is_added())
        {
            transform.set_if_neq(first_person_pose(
                swing.progress(),
                settings.model == SkinModel::Slim,
            ));
        }
    }
    for mut visibility in &mut overlays {
        visibility.set_if_neq(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
    for mut camera in &mut cameras {
        if camera.is_active != visible {
            camera.is_active = visible;
        }
    }
}
