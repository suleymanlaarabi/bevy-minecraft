//! Pose skin parts without changing their geometry or texture data.
use super::mesh::PIXEL;
use super::*;
use crate::{
    character::{CharacterBody, CharacterMotion, CharacterState, Pose},
    player::{Player, PlayerCamera},
};
use bevy::window::PrimaryWindow;
use std::f32::consts::{PI, TAU};

fn part_transform(part: u8, swing: f32, head: Vec2, crouch: bool) -> Transform {
    let pivot = match part {
        0 | 1 => Vec3::new(0., 24., 0.),
        2 => Vec3::new(-6., 24., 0.),
        3 => Vec3::new(6., 24., 0.),
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
    window: Single<&Window, With<PrimaryWindow>>,
    portrait: Single<(&ComputedNode, &UiGlobalTransform), With<SkinPortrait>>,
    mut rigs: Query<(&SkinRig, &mut Transform), Without<SkinPart>>,
    mut parts: Query<(&SkinPart, &ChildOf, &mut Transform), Without<SkinRig>>,
) {
    let (node, transform) = *portrait;
    let (_, _, center) = transform.to_scale_angle_translation();
    let pointer = window.physical_cursor_position().unwrap_or(center);
    let delta = (pointer - center) / node.size().max(Vec2::ONE);
    let head = Vec2::new(delta.x.atan() * 0.65, delta.y.atan() * 0.55);
    for (rigs, mut transforms) in rigs.contiguous_iter_mut().expect("Dense skin rig query") {
        for (index, rig) in rigs.iter().enumerate() {
            if rig.preview {
                let pose = Transform {
                    rotation: Quat::from_rotation_y(-0.22 + head.x * 0.2),
                    ..transforms[index]
                };
                hollow_features::query::set_if_neq(&mut transforms, index, pose);
            }
        }
    }
    for (parts, parents, mut transforms) in
        parts.contiguous_iter_mut().expect("Dense skin part query")
    {
        for (index, (part, parent)) in parts.iter().zip(parents).enumerate() {
            if rigs.get(parent.parent()).is_ok_and(|(rig, _)| rig.preview) {
                hollow_features::query::set_if_neq(
                    &mut transforms,
                    index,
                    part_transform(part.part, 0., head, false),
                );
            }
        }
    }
}

// Rig roots and mesh parts are genuinely disjoint entity sets; parts read their rig
// through `ChildOf`, so the pass is linear in parts.
pub(super) fn animate_world(
    time: Res<Time>,
    view: Res<PlayerView>,
    camera: Single<&PlayerCamera>,
    player: Single<(&CharacterMotion, &CharacterState, &CharacterBody), With<Player>>,
    mut rigs: Query<
        (&mut SkinRig, &mut Transform, &mut Visibility),
        (Without<SkinPart>, Without<FirstPersonArm>),
    >,
    mut parts: Query<
        (&SkinPart, &ChildOf, &mut Transform),
        (Without<SkinRig>, Without<FirstPersonArm>),
    >,
) {
    let (motion, character, body) = *player;
    let hidden = *view == PlayerView::FirstPerson;
    let speed = motion.velocity.xz().length() as f32;
    for (rigs, mut transforms, mut visibility) in rigs
        .contiguous_iter_mut()
        .expect("Dense world skin rig query")
    {
        let (rigs, mut ticks) = ContiguousMut::bypass_change_detection_split(rigs);
        for (index, rig) in rigs.iter_mut().enumerate() {
            if rig.preview {
                continue;
            }
            let target = if hidden {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            hollow_features::query::set_if_neq(&mut visibility, index, target);
            if hidden {
                continue;
            }
            if speed > 0. {
                rig.phase = (rig.phase + time.delta_secs() * speed * 70.).rem_euclid(TAU);
                let row = u32::try_from(index).expect("Rig index must fit in a table row");
                ticks.reborrow().slice(row..row + 1).mark_all_as_changed();
            }
            let mut pose = Transform::from_xyz(0., -body.origin_height, 0.)
                .with_rotation(Quat::from_rotation_y(PI));
            if character.pose == Pose::Swimming {
                pose.rotation *= Quat::from_rotation_x(PI * 0.5);
                pose.translation += Vec3::new(0., 0.3, 0.9);
            }
            hollow_features::query::set_if_neq(&mut transforms, index, pose);
        }
    }
    if hidden {
        return;
    }
    let crouching = character.pose == Pose::Crouching;
    let head = Vec2::new(0., -camera.pitch);
    let amplitude = (speed * 3.).min(0.7);
    for (parts, parents, mut poses) in parts
        .contiguous_iter_mut()
        .expect("Dense world skin part query")
    {
        for (index, (part, parent)) in parts.iter().zip(parents).enumerate() {
            let Ok((rig, ..)) = rigs.get(parent.parent()) else {
                continue;
            };
            if !rig.preview {
                hollow_features::query::set_if_neq(
                    &mut poses,
                    index,
                    part_transform(part.part, rig.phase.sin() * amplitude, head, crouching),
                );
            }
        }
    }
}
