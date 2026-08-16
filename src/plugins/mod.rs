use bevy::app::plugin_group;

pub mod animation;
pub mod core;

use animation::SpriteAnimationPlugin;
use core::GameCorePlugin;

plugin_group! {
    pub struct GamePlugins {
        :SpriteAnimationPlugin,
        :GameCorePlugin
    }
}
