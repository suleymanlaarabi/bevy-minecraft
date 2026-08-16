use bevy::prelude::*;

#[derive(Default)]
pub struct GameCorePlugin;

#[derive(Component, Clone, PartialEq, Eq, Debug)]
#[relationship(relationship_target = FollowedBy)]
pub struct Follow(pub Entity);

impl Default for Follow {
    fn default() -> Self {
        Follow(Entity::PLACEHOLDER)
    }
}

#[derive(Component, Default)]
#[relationship_target(relationship = Follow, linked_spawn)]
pub struct FollowedBy(Vec<Entity>);

impl Plugin for GameCorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, move_follower);
    }
}

fn move_follower(followed: Query<(Entity, &FollowedBy)>, mut transforms: Query<&mut Transform>) {
    for (entity, followers) in &followed {
        let translation = {
            let Ok(transform) = transforms.get_mut(entity) else {
                continue;
            };

            if !transform.is_changed() {
                continue;
            }

            transform.translation
        };

        for &follower in &followers.0 {
            if let Ok(mut transform) = transforms.get_mut(follower) {
                transform.translation = translation;
            }
        }
    }
}
