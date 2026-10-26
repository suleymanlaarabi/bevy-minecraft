use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub(crate) enum GameLayer {
    #[default]
    Default,
    World,
    Player,
    DroppedItem,
}

#[derive(Component, Clone)]
#[relationship(relationship_target = FollowedBy)]
pub struct Follow {
    #[relationship]
    pub entity: Entity,
}

#[derive(Component, Default, Clone)]
pub struct FollowOffset(pub Vec3);

impl Follow {
    pub fn new(entity: Entity) -> Self {
        Self { entity }
    }
}

#[derive(Component, Default, Clone)]
#[relationship_target(relationship = Follow)]
pub struct FollowedBy(Vec<Entity>);

pub struct GameSpatialPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FollowSystems;

impl Plugin for GameSpatialPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            follow_system
                .in_set(FollowSystems)
                .before(TransformSystems::Propagate),
        );
    }
}

fn follow_system(
    followed: Query<(&Transform, &FollowedBy), Without<Follow>>,
    mut follower: Query<(&mut Transform, Option<&FollowOffset>), With<Follow>>,
) {
    for (followed_transform, followed_by) in followed.iter() {
        for follower_entity in followed_by.0.iter() {
            if let Ok((mut follower_transform, follow)) = follower.get_mut(*follower_entity) {
                let translation =
                    followed_transform.translation + follow.map(|f| f.0).unwrap_or_default();
                if follower_transform.translation != translation {
                    follower_transform.translation = translation;
                }
            }
        }
    }
}
