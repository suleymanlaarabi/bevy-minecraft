//! Camera-local hand posing and attack cycles, independent of block destruction time.
use super::*;
use crate::player::PlayerCamera;
use bevy::{camera::CameraProjection, render::render_resource::Extent3d, window::PrimaryWindow};
use std::{f32::consts::PI, sync::OnceLock};

const SWING_TICK_DURATION: f32 = 1. / 20.;
const SWING_DURATION_TICKS: u8 = 6;

#[derive(Component, Default)]
pub(crate) struct ArmSwing {
    ticks: Option<u8>,
    tick_elapsed: f32,
    previous_progress: f32,
    current_progress: f32,
    requested: bool,
    mining_requested: bool,
}

impl ArmSwing {
    pub(crate) fn request(&mut self) {
        self.requested = true;
    }

    pub(crate) fn request_mining(&mut self) {
        self.mining_requested = true;
    }

    fn progress(&self) -> f32 {
        // Minecraft interpolates a restarted swing through the end of the cycle,
        // rather than interpolating backward from the previous pose to zero.
        let end = if self.current_progress < self.previous_progress {
            self.current_progress + 1.
        } else {
            self.current_progress
        };
        (self.previous_progress
            + (end - self.previous_progress) * (self.tick_elapsed / SWING_TICK_DURATION))
            .rem_euclid(1.)
    }

    fn tick(&mut self, requested: bool) {
        // Match LivingEntity.SwingState: a six-tick swing can restart once its
        // counter exceeds three ticks. Repeated mining therefore restarts at four.
        if requested
            && self
                .ticks
                .is_none_or(|ticks| ticks == 0 || ticks > SWING_DURATION_TICKS / 2)
        {
            if self.ticks.is_none() {
                self.previous_progress = 0.;
                self.current_progress = 0.;
            }
            self.ticks = Some(0);
        }

        self.previous_progress = self.current_progress;
        if let Some(ticks) = self.ticks {
            if ticks > SWING_DURATION_TICKS {
                self.ticks = None;
                self.current_progress = 0.;
            } else {
                self.current_progress = f32::from(ticks) / f32::from(SWING_DURATION_TICKS);
                self.ticks = Some(ticks + 1);
            }
        } else {
            self.current_progress = 0.;
        }
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
    if swing.ticks.is_none()
        && swing.previous_progress == 0.
        && swing.current_progress == 0.
        && !swing.requested
        && !swing.mining_requested
        && !clicked
    {
        return;
    }
    // Keep one-shot clicks until the next animation tick, but sample mining anew
    // each frame so releasing the button or losing the target stops repetition.
    swing.requested |= clicked;
    let mining = std::mem::take(&mut swing.mining_requested);
    swing.tick_elapsed += time.delta_secs();
    while swing.tick_elapsed >= SWING_TICK_DURATION {
        swing.tick_elapsed -= SWING_TICK_DURATION;
        let requested = std::mem::take(&mut swing.requested) || mining;
        swing.tick(requested);
    }
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
    arms: Query<(Entity, &Transform, &Visibility), (With<FirstPersonRoot>, Without<ArmView>)>,
    parts: Query<(&Mesh3d, &Transform, &ChildOf), Or<(With<SkinPart>, With<HeldItemPart>)>>,
    mut cameras: Query<(&mut Camera, &Projection), With<ArmCamera>>,
    mut overlays: Query<(&mut Node, &mut Visibility), (With<ArmView>, Without<FirstPersonRoot>)>,
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
            for (root, arm, visibility) in &arms {
                if *visibility == Visibility::Hidden {
                    continue;
                }
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
    held: Single<Ref<HeldItem>>,
    mut arms: Query<
        (&mut Transform, &mut Visibility, Option<&HeldItem>),
        (With<FirstPersonRoot>, Without<ArmView>),
    >,
    mut overlays: Query<&mut Visibility, (With<ArmView>, Without<FirstPersonRoot>)>,
    mut cameras: Query<&mut Camera, With<ArmCamera>>,
) {
    // Mesh rig roots and UI image roots are genuinely disjoint entity sets.
    let visible = *state.get() == InventoryState::Closed && *view == PlayerView::FirstPerson;
    for (mut transform, mut visibility, item) in &mut arms {
        let shown = visible
            && if item.is_some() {
                held.item.is_some()
            } else {
                held.item.is_none()
            };
        let became_visible = shown && *visibility == Visibility::Hidden;
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if shown
            && (became_visible
                || swing.is_changed()
                || settings.is_changed()
                || view.is_changed()
                || held.is_changed()
                || transform.is_added())
        {
            let pose = if let Some(item) = item {
                held_item::pose(swing.progress(), item.equipped, &item.display)
            } else {
                let mut pose =
                    first_person_pose(swing.progress(), settings.model == SkinModel::Slim);
                pose.translation.y -= 0.6 * (1. - held.equipped);
                pose
            };
            transform.set_if_neq(pose);
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
