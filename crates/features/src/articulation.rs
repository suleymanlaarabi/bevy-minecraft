//! Lightweight articulated voxel parts: shared clips and active-instance updates.
use bevy_app::{App, Plugin, PostUpdate};
use bevy_asset::{Asset, AssetApp, Assets, Handle};
use bevy_ecs::{prelude::*, system::ParallelCommands};
use bevy_math::prelude::*;
use bevy_reflect::TypePath;
use bevy_time::Time;
use bevy_transform::{TransformSystems, components::Transform};

#[derive(Clone, Copy)]
pub enum JointMotion {
    Rotation { axis: Dir3, radians: f32 },
    Translation { start: Vec3, end: Vec3 },
    Scale { start: Vec3, end: Vec3 },
}
#[derive(Clone, Copy)]
pub enum Easing {
    Linear,
    CubicOut,
}

/// Elementary joints need one channel, avoiding a per-instance animation graph.
#[derive(Asset, TypePath)]
pub struct JointClip {
    pub motion: JointMotion,
    pub duration_seconds: f32,
    pub easing: Easing,
}
#[derive(Component)]
pub struct JointAnimation {
    pub clip: Handle<JointClip>,
    pub progress: f32,
    pub target: f32,
}
impl JointAnimation {
    pub fn new(clip: Handle<JointClip>) -> Self {
        Self {
            clip,
            progress: 0.0,
            target: 0.0,
        }
    }
}
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct AnimatingJoint;

/// Applied on changes; dormant joints do not participate in animation iteration.
#[derive(EntityEvent)]
pub struct SetJointTarget {
    pub entity: Entity,
    pub target: f32,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArticulationSystems;

pub struct ArticulationPlugin;
impl Plugin for ArticulationPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<JointClip>()
            .add_observer(set_target)
            .add_systems(
                PostUpdate,
                animate
                    .in_set(ArticulationSystems)
                    .before(TransformSystems::Propagate),
            );
    }
}
fn set_target(
    event: On<SetJointTarget>,
    mut commands: Commands,
    mut joints: Query<&mut JointAnimation>,
) {
    if !event.target.is_finite() {
        return;
    }
    if let Ok(mut joint) = joints.get_mut(event.entity) {
        let target = event.target.clamp(0.0, 1.0);
        if joint.target != target {
            joint.target = target;
            commands.entity(event.entity).insert(AnimatingJoint);
        }
    }
}
fn animate(
    commands: ParallelCommands,
    time: Res<Time>,
    clips: Res<Assets<JointClip>>,
    mut joints: Query<(Entity, &mut JointAnimation, &mut Transform), With<AnimatingJoint>>,
) {
    joints
        .par_iter_mut()
        .for_each(|(entity, mut joint, mut transform)| {
            let Some(clip) = clips.get(&joint.clip) else {
                return;
            };
            let step = if clip.duration_seconds > 0.0 {
                time.delta_secs() / clip.duration_seconds
            } else {
                1.0
            };
            joint.progress = joint.progress.move_towards(joint.target, step);
            let progress = match clip.easing {
                Easing::Linear => joint.progress,
                Easing::CubicOut => 1.0 - (1.0 - joint.progress).powi(3),
            };
            match clip.motion {
                JointMotion::Rotation { axis, radians } => {
                    transform.rotation = Quat::from_axis_angle(*axis, progress * radians)
                }
                JointMotion::Translation { start, end } => {
                    transform.translation = start.lerp(end, progress)
                }
                JointMotion::Scale { start, end } => transform.scale = start.lerp(end, progress),
            }
            if joint.progress == joint.target {
                commands.command_scope(|mut commands| {
                    commands.entity(entity).remove::<AnimatingJoint>();
                });
            }
        });
}
