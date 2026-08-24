use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{
    character::CharacterPlugin, menu::GameMenuPlugin, player::PlayerPlugin,
    spatial::GameSpatialPlugin,
};

#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub enum GameState {
    #[default]
    Menu,
    Settings,
    Game,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_state(GameState::Menu)
            .add_plugins((
                PlayerPlugin,
                GameMenuPlugin,
                CharacterPlugin,
                GameSpatialPlugin,
            ))
            .add_systems(OnEnter(GameState::Menu), setup_menu_environment)
            .add_systems(OnEnter(GameState::Settings), setup_menu_environment)
            .add_systems(OnEnter(GameState::Game), setup_game_environment);
    }
}

fn setup_menu_environment(
    mut cursors: Query<&mut CursorOptions>,
    mut clear_color: ResMut<ClearColor>,
) {
    for mut cursor in &mut cursors {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
    clear_color.0 = Color::srgb_u8(15, 15, 18);
}

fn setup_game_environment(
    mut cursors: Query<&mut CursorOptions>,
    mut clear_color: ResMut<ClearColor>,
    mut commands: Commands,
) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            shadow_depth_bias: 0.02,
            shadow_normal_bias: 1.8,
            illuminance: 5_000.0,
            color: Color::srgb(1.0, 0.98, 0.94),
            ..default()
        },
        Transform::from_xyz(100.0, 200.0, 100.0).looking_at(Vec3::ZERO, Vec3::Y),
        DespawnOnExit(GameState::Game),
    ));

    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    clear_color.0 = Color::srgb_u8(148, 195, 255);
}
