//! Translation followers compose a target relationship and a mutable local offset.
use bevy_app::{App, Plugin, PostUpdate};
use bevy_ecs::prelude::*;
use bevy_math::Vec3;
use bevy_transform::{TransformSystems, components::Transform};

#[derive(Component, Clone)]
#[relationship(relationship_target = FollowedBy)]
#[require(Transform, FollowOffset)]
pub struct Follow {
    #[relationship]
    pub entity: Entity,
}

#[derive(Component, Default, Clone)]
pub struct FollowOffset(pub Vec3);

// Targets can have several followers; Bevy owns this canonical reverse membership.
#[derive(Component, Default, Clone)]
#[relationship_target(relationship = Follow)]
pub struct FollowedBy(Vec<Entity>);

pub struct FollowPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FollowSystems;

impl Plugin for FollowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            follow_system
                .in_set(FollowSystems)
                .before(TransformSystems::Propagate),
        );
    }
}

/// Targets are roots: follower chains are deliberately excluded from this pass.
fn follow_system(
    targets: Query<&Transform, Without<Follow>>,
    mut followers: Query<(&Follow, &FollowOffset, &mut Transform)>,
) {
    for (follows, offsets, mut transforms) in followers
        .contiguous_iter_mut()
        .expect("Dense follower query")
    {
        for (index, (follow, offset)) in follows.iter().zip(offsets).enumerate() {
            let Ok(target) = targets.get(follow.entity) else {
                continue;
            };
            crate::query::set_field_if_neq(
                &mut transforms,
                index,
                |transform| &mut transform.translation,
                target.translation + offset.0,
            );
        }
    }
}
