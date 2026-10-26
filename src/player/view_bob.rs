//! Minecraft Java 26.3 GameRenderer.bobView, expressed as a view-space transform.
use crate::character::CharacterMotion;
use bevy::prelude::*;
use std::f32::consts::PI;

pub(super) fn pose(motion: &CharacterMotion) -> Transform {
    let (distance, bob) = motion.walking_animation();
    let phase = distance * PI;
    let (sin, cos) = phase.sin_cos();
    Transform::from_xyz(sin * bob * 0.5, -(cos * bob).abs(), 0.0).with_rotation(
        Quat::from_rotation_z((sin * bob * 3.0).to_radians())
            * Quat::from_rotation_x(((phase - 0.2).cos() * bob).abs().to_radians() * 5.0),
    )
}
