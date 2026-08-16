use avian2d::{
    PhysicsPlugins, collision::collider::Collider, debug_render::PhysicsDebugPlugin,
    dynamics::rigid_body::RigidBody,
};
use bevy::prelude::*;

use crate::plugins::{GamePlugins, player::Player};

mod plugins;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(ImagePlugin::default_nearest()),
            PhysicsPlugins::default(),
            PhysicsDebugPlugin,
            GamePlugins,
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn_scene(Player::scene());
    commands.spawn((
        Transform::from_translation(Vec3::ZERO.with_y(-200.)),
        Collider::rectangle(500., 10.),
        RigidBody::Static,
    ));
}
