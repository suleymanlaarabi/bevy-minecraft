//! Local player composition; systems are registered here in their original schedules.

mod components;
mod input;
mod interaction;
mod safety;
mod spawn;

use crate::{game::GameState, inventory::InventoryState};
use bevy::prelude::*;
use input::{clear_player_input, mouse_look, player_input};
use spawn::{finish_spawn_loading, finish_spawn_search, start_spawn_search};

pub(crate) use components::Crosshair;
pub use components::{CameraSensitivity, Player, PlayerCamera};

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(GameState::Game),
            start_spawn_search.after(crate::world::PrepareSession),
        )
        .add_systems(
            Update,
            (finish_spawn_search, finish_spawn_loading)
                .chain()
                .run_if(in_state(crate::world::WorldSessionPhase::Loading)),
        )
        .add_systems(OnEnter(InventoryState::Open), clear_player_input)
        .add_systems(
            RunFixedMainLoop,
            (mouse_look, player_input)
                .chain()
                .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
                .run_if(crate::world::playing)
                .run_if(in_state(InventoryState::Closed)),
        );
        interaction::register(app);
        safety::register(app);
    }
}
