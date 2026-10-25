use bevy::{
    camera::visibility::RenderLayers,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{
    character::CharacterPlugin,
    dropped_item::DroppedItemPlugin,
    inventory::{InventoryPlugin, InventoryState},
    menu::GameMenuPlugin,
    player::PlayerPlugin,
    spatial::GameSpatialPlugin,
    voxel::{SHADOW_LAYER, SUN_POSITION},
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

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_state(GameState::Menu)
            .add_plugins((
                crate::world::WorldPlugin,
                InventoryPlugin,
                PlayerPlugin,
                GameMenuPlugin,
                CharacterPlugin,
                GameSpatialPlugin,
                DroppedItemPlugin,
            ))
            .add_systems(OnEnter(GameState::Menu), setup_menu_environment)
            .add_systems(OnEnter(GameState::Settings), setup_menu_environment)
            .add_systems(OnEnter(GameState::WorldSelect), setup_menu_environment)
            .add_systems(OnEnter(GameState::WorldCreate), setup_menu_environment)
            .add_systems(OnEnter(GameState::Game), setup_game_environment);
    }
}

fn setup_menu_environment(
    state: Res<State<GameState>>,
    mut cursors: Query<&mut CursorOptions>,
    mut clear_color: ResMut<ClearColor>,
    mut commands: Commands,
) {
    commands.spawn((Camera2d, IsDefaultUiCamera, DespawnOnExit(*state.get())));

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
        Transform::from_translation(SUN_POSITION).looking_at(Vec3::ZERO, Vec3::Y),
        // Lights the camera's layer and casts terrain shadows from the shadow layer.
        RenderLayers::from_layers(&[0, SHADOW_LAYER]),
        DespawnOnExit(GameState::Game),
    ));

    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    clear_color.0 = Color::srgb_u8(148, 195, 255);
}

pub(crate) fn handle_escape(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    inventory_state: Res<State<InventoryState>>,
    mut next_inventory_state: ResMut<NextState<InventoryState>>,
    mut commands: Commands,
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
        crate::world::request_leave(commands.reborrow());
    }
}
