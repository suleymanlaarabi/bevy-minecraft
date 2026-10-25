#![allow(clippy::type_complexity)]

use super::{CameraSensitivity, Player, PlayerCamera};
use crate::character::CharacterMovement;
use bevy::{input::mouse::AccumulatedMouseMotion, prelude::*};
use std::f32::consts::FRAC_PI_2;

pub(super) fn clear_player_input(mut players: Query<&mut CharacterMovement, With<Player>>) {
    for mut movement in &mut players {
        *movement = CharacterMovement::default();
    }
}

pub(super) fn mouse_look(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerCamera>)>,
    mut camera: Single<
        (&mut Transform, &mut PlayerCamera, &CameraSensitivity),
        (With<PlayerCamera>, Without<Player>),
    >,
) {
    let delta = accumulated_mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    let (camera_transform, player_camera, sensitivity) = &mut *camera;
    player.rotate_y(-delta.x * sensitivity.x);

    const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;
    player_camera.pitch =
        (player_camera.pitch - delta.y * sensitivity.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    camera_transform.rotation = player.rotation * Quat::from_rotation_x(player_camera.pitch);
}

pub(super) fn player_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    camera: Single<&Transform, With<PlayerCamera>>,
    player: Single<(&Transform, &mut CharacterMovement), With<Player>>,
) {
    let (transform, mut movement) = player.into_inner();

    let forward = transform.forward().as_vec3();
    let forward_flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();

    let right = transform.right().as_vec3();
    let right_flat = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

    movement.direction = (forward_flat * axis(&keyboard, KeyCode::KeyW, KeyCode::KeyS)
        + right_flat * axis(&keyboard, KeyCode::KeyD, KeyCode::KeyA))
    .normalize_or_zero();

    movement.look_direction = camera.forward().as_vec3();

    movement.jump = keyboard.pressed(KeyCode::Space);
    movement.sneak = keyboard.pressed(KeyCode::ShiftLeft);
    movement.sprint = keyboard.pressed(KeyCode::ControlLeft);
}

fn axis(input: &ButtonInput<KeyCode>, positive: KeyCode, negative: KeyCode) -> f32 {
    (input.pressed(positive) as i8 - input.pressed(negative) as i8) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn opening_inventory_clears_all_movement_commands() {
        let mut world = World::new();
        let player = world
            .spawn((
                Player,
                CharacterMovement {
                    direction: Vec3::X,
                    look_direction: Vec3::X,
                    jump: true,
                    sprint: true,
                    sneak: true,
                },
            ))
            .id();
        world.run_system_once(clear_player_input).unwrap();
        let movement = world.get::<CharacterMovement>(player).unwrap();
        assert_eq!(movement.direction, Vec3::ZERO);
        assert!(!movement.jump && !movement.sprint && !movement.sneak);
    }
}
