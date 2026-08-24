use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{character::CharacterPlugin, menu::GameMenuPlugin, player::PlayerPlugin};

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
            .add_plugins((PlayerPlugin, GameMenuPlugin, CharacterPlugin))
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
) {
    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    clear_color.0 = Color::srgb_u8(148, 195, 255);
}
