use avian2d::{
    collision::collider::Collider,
    dynamics::rigid_body::{RigidBody, mass_properties::components::ColliderDensity},
};
use bevy::{image::TextureAtlasTemplate, prelude::*};

use crate::plugins::{
    animation::{SpriteAnimationIndices, SpriteAnimationTimer},
    core::FollowedBy,
};

#[derive(SceneComponent, Default, Clone)]
pub struct Player;

impl Player {
    pub fn scene() -> impl Scene {
        bsn! {
            Transform::from_scale(Vec3::splat(2.))
            Sprite {
                image: "idle.png",
                texture_atlas: Option::<TextureAtlasTemplate>::Some(
                    TextureAtlasTemplate {
                        layout: asset_value(TextureAtlasLayout::from_grid(UVec2::new(46, 55), 10, 1, None, None)),
                        index: 0,
                    }
                )
            }
            SpriteAnimationTimer(Timer::from_seconds(0.1, TimerMode::Repeating))
            SpriteAnimationIndices::new(0, 9)
            template_value(RigidBody::Dynamic)
            Collider::capsule(10., 30.)
            ColliderDensity(10.)
            FollowedBy [Camera2d]
        }
    }
}
