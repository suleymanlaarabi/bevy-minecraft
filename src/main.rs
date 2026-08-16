use bevy::{image::TextureAtlasTemplate, prelude::*};

use crate::plugins::{
    GamePlugins,
    animation::{SpriteAnimationIndices, SpriteAnimationTimer},
    core::FollowedBy,
};

mod plugins;

#[derive(SceneComponent, Default, Clone)]
struct Player;

impl Player {
    fn scene() -> impl Scene {
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
            FollowedBy [Camera2d]
        }
    }
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(ImagePlugin::default_nearest()),
            GamePlugins,
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn_scene(Player::scene());
}
