#![allow(clippy::type_complexity)]

use super::{CameraSensitivity, Player, PlayerCamera};
use crate::{
    character::{CharacterMotion, CharacterMovement},
    spatial::FollowOffset,
};
use bevy::{input::mouse::AccumulatedMouseMotion, prelude::*};
use std::f32::consts::FRAC_PI_2;

pub(crate) fn clear_player_input(mut players: Query<&mut CharacterMovement, With<Player>>) {
    for mut movement in &mut players {
        movement.set_if_neq(CharacterMovement::default());
    }
}

pub(super) fn mouse_look(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    mut player: Single<&mut Transform, With<Player>>,
    mut camera: Single<(&mut PlayerCamera, &CameraSensitivity)>,
) {
    let delta = accumulated_mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    // The camera rotation itself is applied by `follow_eye_height`.
    let (player_camera, sensitivity) = &mut *camera;
    player.rotate_y(-delta.x * sensitivity.x);

    const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;
    player_camera.pitch =
        (player_camera.pitch - delta.y * sensitivity.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
}

pub(super) fn player_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    camera: Single<Ref<PlayerCamera>>,
    player: Single<(Ref<Transform>, &mut CharacterMovement), With<Player>>,
) {
    let (transform, mut movement) = player.into_inner();
    if !keyboard.is_changed()
        && !camera.is_changed()
        && !transform.is_changed()
        && !movement.is_changed()
    {
        return;
    }

    let forward = transform.forward().as_vec3();
    let forward_flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();

    let right = transform.right().as_vec3();
    let right_flat = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

    movement.set_if_neq(CharacterMovement {
        direction: (forward_flat * axis(&keyboard, KeyCode::KeyW, KeyCode::KeyS)
            + right_flat * axis(&keyboard, KeyCode::KeyD, KeyCode::KeyA))
        .normalize_or_zero(),
        look_direction: camera.look(transform.rotation) * Vec3::NEG_Z,
        jump: keyboard.pressed(KeyCode::Space),
        sneak: keyboard.pressed(KeyCode::ShiftLeft),
        sprint: keyboard.pressed(KeyCode::ControlLeft),
    });
}

/// Crouching and swimming lower the camera smoothly, as the eyes ease between poses.
pub(super) fn follow_eye_height(
    player: Single<
        (
            &CharacterMotion,
            &Transform,
            &crate::survival::DamageState,
            &crate::survival::ActiveEffects,
        ),
        (With<Player>, Without<PlayerCamera>),
    >,
    mut camera: Single<(&mut FollowOffset, &mut Transform, &PlayerCamera), Without<Player>>,
    view: Res<super::skin::PlayerView>,
    world: crate::voxel::VoxelWorld,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    settings: Res<crate::settings::GraphicsSettings>,
) {
    use super::skin::PlayerView;
    let (motion, player_transform, damage, effects) = *player;
    let (offset, camera_transform, player_camera) = &mut *camera;
    let look = player_camera.look(player_transform.rotation);
    // Minecraft multiplies the view matrix by bobView; the camera world pose
    // therefore needs its inverse. Hands use bobView directly in camera space.
    let bob_pose = super::view_bob::pose(motion);
    let bob_rotation = bob_pose.rotation.inverse();
    let mut eye = Vec3::Y * motion.eye_offset();
    if *view == PlayerView::FirstPerson {
        eye -= look * (bob_rotation * bob_pose.translation);
    }
    if *view != PlayerView::FirstPerson {
        let direction = look
            * if *view == PlayerView::Front {
                Vec3::NEG_Z
            } else {
                Vec3::Z
            };
        let mut distance = 0.0;
        // Keep the camera in loaded air, including a small margin around its near plane
        // (sampled in the plane perpendicular to the view direction).
        for step in 1..=30 {
            let next = step as f32 * 0.1;
            let position = player_transform.translation + eye + direction * next;
            let blocked = [-0.12, 0.12].into_iter().any(|x| {
                [-0.12, 0.12].into_iter().any(|y| {
                    world
                        .get_at(position + look * Vec3::new(x, y, 0.))
                        .is_none_or(|kind| kind.is_solid())
                })
            });
            if blocked {
                break;
            }
            distance = next;
        }
        eye += direction * distance;
    }
    if offset.0 != eye {
        offset.0 = eye;
    }
    let mut rotation = if *view == PlayerView::Front {
        look * Quat::from_rotation_y(std::f32::consts::PI)
    } else if *view == PlayerView::FirstPerson {
        look * bob_rotation
    } else {
        look
    };
    if settings.damage_camera_motion {
        let remaining = (damage.flash_ticks as f32 - fixed.overstep_fraction()).max(0.0) / 10.0;
        let hurt = ((1.0 - remaining).powi(4) * std::f32::consts::PI).sin() * remaining * 0.18;
        let nausea = if effects.level(pumpkin_data::effect::StatusEffect::NAUSEA.id) > 0 {
            (time.elapsed_secs() * 1.8).sin() * 0.015
        } else {
            0.0
        };
        rotation *= Quat::from_rotation_z(hurt + nausea);
    }
    if camera_transform.rotation != rotation {
        camera_transform.rotation = rotation;
    }
}

fn axis(input: &ButtonInput<KeyCode>, positive: KeyCode, negative: KeyCode) -> f32 {
    (input.pressed(positive) as i8 - input.pressed(negative) as i8) as f32
}
