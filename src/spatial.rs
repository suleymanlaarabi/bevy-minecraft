use bevy::prelude::*;

#[derive(Component, Clone)]
#[relationship(relationship_target = FollowedBy)]
pub struct Follow {
    #[relationship]
    pub entity: Entity,
    pub offset: Vec3,
}

#[derive(Component, Default, Clone)]
pub struct FollowOffset(pub Vec3);

impl Follow {
    pub fn new(entity: Entity, offset: Vec3) -> Self {
        Self { entity, offset }
    }
}

#[derive(Component, Default, Clone)]
#[relationship_target(relationship = Follow)]
pub struct FollowedBy(Vec<Entity>);

pub struct GameSpatialPlugin;

impl Plugin for GameSpatialPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, follow_system);
    }
}

fn follow_system(
    followed: Query<(&Transform, &FollowedBy), Without<Follow>>,
    mut follower: Query<(&mut Transform, Option<&FollowOffset>), With<Follow>>,
) {
    for (followed_transform, followed_by) in followed.iter() {
        for follower_entity in followed_by.0.iter() {
            if let Ok((mut follower_transform, follow)) = follower.get_mut(*follower_entity) {
                follower_transform.translation =
                    followed_transform.translation + follow.map(|f| f.0).unwrap_or_default();
            }
        }
    }
}
