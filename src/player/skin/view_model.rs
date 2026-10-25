//! Camera-local hand posing and attack cycles, independent of block destruction time.
use super::*;
use crate::player::PlayerCamera;
use bevy::{render::render_resource::Extent3d, window::PrimaryWindow};
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

pub(super) fn resize_arm_target(
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<SkinAssets>,
    mut images: ResMut<Assets<Image>>,
) {
    let size = UVec2::new(
        window.physical_width().max(1),
        window.physical_height().max(1),
    );
    if let Some(image) = images.get(&assets.arm_view)
        && image.size() != size
        && let Some(mut image) = images.get_mut(&assets.arm_view)
    {
        image.resize(Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
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
