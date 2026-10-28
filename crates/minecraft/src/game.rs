use bevy::prelude::*;

use crate::{
    character::CharacterPlugin,
    devtools::DevToolsPlugin,
    dropped_item::DroppedItemPlugin,
    inventory::{InventoryPlugin, InventoryState},
    menu::GameMenuPlugin,
    player::PlayerPlugin,
    sky::SkyPlugin,
};

#[derive(Clone, Copy, Default, Eq, PartialEq, Debug, Hash, States)]
pub enum GameState {
    #[default]
    Menu,
    Settings,
    WorldSelect,
    WorldCreate,
    Game,
}

/// Declarative visibility for gameplay HUD roots, independent of their content plugin.
#[derive(Component, Default, Clone, Copy)]
#[require(Visibility)]
pub struct GameplayHud {
    pub hide_with_inventory: bool,
}

fn sync_gameplay_hud(
    phase: Option<Res<State<crate::world::WorldSessionPhase>>>,
    inventory: Option<Res<State<InventoryState>>>,
    mut roots: Query<(Ref<GameplayHud>, &mut Visibility)>,
) {
    let playing = phase
        .as_ref()
        .is_some_and(|phase| *phase.get() == crate::world::WorldSessionPhase::Playing);
    let open = inventory
        .as_ref()
        .is_some_and(|state| *state.get() == InventoryState::Open);
    let changed = phase.as_ref().is_some_and(|phase| phase.is_changed())
        || inventory.as_ref().is_some_and(|state| state.is_changed());
    for (policy, mut visibility) in &mut roots {
        if changed || policy.is_changed() || visibility.is_added() {
            visibility.set_if_neq(if playing && !(open && policy.hide_with_inventory) {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_state(GameState::Menu)
            .add_plugins((
                crate::world::WorldPlugin,
                crate::simulation::GameplaySimulationPlugin,
                InventoryPlugin,
                crate::audio::GameAudioPlugin,
                crate::block_behavior::BlockBehaviorPlugins,
                PlayerPlugin,
                GameMenuPlugin,
                hollow_features::motion::FixedPositionPlugin,
                CharacterPlugin,
                crate::survival::SurvivalPlugin,
                hollow_features::spatial::FollowPlugin,
                hollow_features::cursor::CursorCapturePlugin,
                DroppedItemPlugin,
                SkyPlugin,
                DevToolsPlugin,
            ))
            .add_plugins(crate::creatures::CreaturesPlugin)
            .configure_sets(
                RunFixedMainLoop,
                hollow_features::motion::InterpolatePosition.run_if(crate::simulation::running),
            )
            .add_systems(
                PostUpdate,
                sync_gameplay_hud
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            )
            .add_systems(OnEnter(GameState::Menu), setup_menu_environment)
            .add_systems(OnEnter(GameState::Settings), setup_menu_environment)
            .add_systems(OnEnter(GameState::WorldSelect), setup_menu_environment)
            .add_systems(OnEnter(GameState::WorldCreate), setup_menu_environment)
            .add_systems(OnEnter(GameState::Game), setup_game_environment);
    }
}

fn setup_menu_environment(
    state: Res<State<GameState>>,
    mut clear_color: ResMut<ClearColor>,
    mut commands: Commands,
) {
    commands.spawn((
        Camera2d,
        IsDefaultUiCamera,
        hollow_features::cursor::FreeCursor,
        DespawnOnExit(*state.get()),
    ));

    clear_color.0 = Color::srgb_u8(15, 15, 18);
}

fn setup_game_environment(mut clear_color: ResMut<ClearColor>) {
    // The atmosphere adds the sky over a black background (`crate::sky`).
    clear_color.0 = Color::BLACK;
}

pub(crate) fn handle_escape(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    inventory_state: Res<State<InventoryState>>,
    mut next_inventory_state: ResMut<NextState<InventoryState>>,
    mut save: ResMut<crate::world::SaveState>,
    phase: Res<State<crate::world::WorldSessionPhase>>,
) {
    if !keyboard_input.just_pressed(KeyCode::Escape) {
        return;
    }
    if !matches!(
        phase.get(),
        crate::world::WorldSessionPhase::Playing | crate::world::WorldSessionPhase::Loading
    ) {
        return;
    }

    if *inventory_state.get() == InventoryState::Open {
        next_inventory_state.set(InventoryState::Closed);
    } else {
        save.exit_requested = true;
    }
}
