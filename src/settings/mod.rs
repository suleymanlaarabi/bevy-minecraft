//! Graphics settings model, UI and engine application.

mod apply;
mod model;
mod ui;

use crate::{game::GameState, voxel::TexturePackId};
use bevy::{input::common_conditions::input_just_pressed, prelude::*, settings::SettingsPlugin};
use ui::spawn_settings_page;

pub(crate) use ui::settings_panel;

pub use model::{AntiAliasingMode, DisplayMode, GraphicsSettings, ShadowQuality};

const SETTINGS_APP_NAME: &str = "org.hollow.game";
/// Debounce before writing settings changed from the UI.
pub(crate) const SAVE_DELAY: core::time::Duration = core::time::Duration::from_millis(250);

pub struct GameSettingsPlugin;

impl Plugin for GameSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<AntiAliasingMode>()
            .register_type::<ShadowQuality>()
            .register_type::<DisplayMode>()
            .register_type::<TexturePackId>()
            .register_type::<GraphicsSettings>();
        apply::register(app);
        app.add_plugins(SettingsPlugin::new(SETTINGS_APP_NAME))
            .add_systems(OnEnter(GameState::Settings), spawn_settings_page)
            .add_systems(
                Update,
                back_to_menu
                    .run_if(in_state(GameState::Settings))
                    .run_if(input_just_pressed(KeyCode::Escape)),
            );
    }
}

fn back_to_menu(mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::Menu);
}
