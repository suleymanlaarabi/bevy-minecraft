use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{menu::GameMenuPlugin, player::PlayerPlugin};

#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub enum GameState {
    #[default]
    Menu,
    Game,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_state(GameState::Menu)
            .add_plugins((PlayerPlugin, GameMenuPlugin))
            .add_systems(OnEnter(GameState::Menu), setup_menu_cursor)
            .add_systems(OnEnter(GameState::Game), setup_game_cursor);
    }
}

fn setup_menu_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut cursor in &mut cursors {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

fn setup_game_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
}
