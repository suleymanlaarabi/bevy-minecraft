//! Pose skin parts without changing their geometry or texture data.
use super::mesh::PIXEL;
use super::*;
use crate::{
    character::{CharacterBody, CharacterMotion, CharacterState, Pose},
    player::{Player, PlayerCamera},
};
use bevy::window::PrimaryWindow;
use std::f32::consts::{PI, TAU};

fn part_transform(part: usize, slim: bool, swing: f32, head: Vec2, crouch: bool) -> Transform {
    let arm_x = if slim { 5.5 } else { 6. };
    let pivot = match part {
        0 | 1 => Vec3::new(0., 24., 0.),
        2 => Vec3::new(-arm_x, 24., 0.),
        3 => Vec3::new(arm_x, 24., 0.),
        4 => Vec3::new(-2., 12., 0.),
        _ => Vec3::new(2., 12., 0.),
    };
    let mut transform = Transform::from_translation(
        (pivot
            - if crouch && part < 4 {
                Vec3::Y * 4.
            } else {
                Vec3::ZERO
            })
            * PIXEL,
    );
    transform.rotation = match part {
        0 => Quat::from_euler(EulerRot::YXZ, head.x, head.y, 0.),
        1 if crouch => Quat::from_rotation_x(0.35),
        2 | 5 => Quat::from_rotation_x(swing),
        3 | 4 => Quat::from_rotation_x(-swing),
        _ => Quat::IDENTITY,
    };
    transform
}

// Rig roots and mesh parts are disjoint; each Transform is borrowed once.
pub(super) fn animate_rigs(
    settings: Res<SkinSettings>,
    window: Single<&Window, With<PrimaryWindow>>,
    portrait: Single<(&ComputedNode, &UiGlobalTransform), With<SkinPortrait>>,
    mut rigs: Query<(&SkinRig, &mut Transform), Without<SkinPart>>,
    mut parts: Query<(&SkinPart, &mut Transform), Without<SkinRig>>,
) {
    let (node, transform) = *portrait;
    let (_, _, center) = transform.to_scale_angle_translation();
    let pointer = window.physical_cursor_position().unwrap_or(center);
    let delta = (pointer - center) / node.size().max(Vec2::ONE);
    let head = Vec2::new(delta.x.atan() * 0.65, delta.y.atan() * 0.55);
    for (rig, mut transform) in &mut rigs {
        if rig.preview {
            transform.set_if_neq(Transform {
                rotation: Quat::from_rotation_y(-0.22 + head.x * 0.2),
                ..*transform
            });
        }
    }
    for (part, mut transform) in &mut parts {
        if rigs.get(part.rig).is_ok_and(|(rig, _)| rig.preview) {
            transform.set_if_neq(part_transform(
                part.part,
                settings.model == SkinModel::Slim,
                0.,
                head,
                false,
            ));
        }
    }
}

// Rig roots and mesh parts are genuinely disjoint entity sets; parts read their rig
// through `SkinPart::rig`, so the pass is linear in parts.
pub(super) fn animate_world(
    time: Res<Time>,
    settings: Res<SkinSettings>,
    view: Res<PlayerView>,
    camera: Single<&PlayerCamera>,
    player: Single<(&CharacterMotion, &CharacterState, &CharacterBody), With<Player>>,
    mut rigs: Query<
        (&mut SkinRig, &mut Transform, &mut Visibility),
        (Without<SkinPart>, Without<FirstPersonArm>),
    >,
    mut parts: Query<(&SkinPart, &mut Transform), (Without<SkinRig>, Without<FirstPersonArm>)>,
) {
    let (motion, character, body) = *player;
    let hidden = *view == PlayerView::FirstPerson;
    let speed = motion.velocity.xz().length() as f32;
    for (mut rig, mut transform, mut visibility) in &mut rigs {
        if rig.preview {
            continue;
        }
        visibility.set_if_neq(if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
        if hidden {
            continue;
        }
        if speed > 0. {
            rig.phase = (rig.phase + time.delta_secs() * speed * 70.).rem_euclid(TAU);
        }
        let mut pose = Transform::from_xyz(0., -body.origin_height, 0.)
            .with_rotation(Quat::from_rotation_y(PI));
        if character.pose == Pose::Swimming {
            pose.rotation *= Quat::from_rotation_x(PI * 0.5);
            pose.translation += Vec3::new(0., 0.3, 0.9);
        }
        transform.set_if_neq(pose);
    }
    if hidden {
        return;
    }
    let slim = settings.model == SkinModel::Slim;
    let crouching = character.pose == Pose::Crouching;
    let head = Vec2::new(0., -camera.pitch);
    let amplitude = (speed * 3.).min(0.7);
    for (part, mut part_pose) in &mut parts {
        let Ok((rig, ..)) = rigs.get(part.rig) else {
            continue;
        };
        if !rig.preview {
            part_pose.set_if_neq(part_transform(
                part.part,
                slim,
                rig.phase.sin() * amplitude,
                head,
                crouching,
            ));
        }
    }
}
