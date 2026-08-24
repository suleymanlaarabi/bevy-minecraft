mod model;
mod ui;

use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*};

use crate::{game::GameState, player::Player};

pub use model::{
    BLOCK_STACK_SIZE, DragMode, HOTBAR_SIZE, INVENTORY_SIZE, ItemKind, ItemStack,
    MAIN_INVENTORY_SIZE, PlayerInventory,
};

#[derive(SubStates, Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[source(GameState = GameState::Game)]
pub enum InventoryState {
    #[default]
    Closed,
    Open,
}

#[derive(Clone, Debug)]
pub(crate) struct DragState {
    pub source_entity: Entity,
    pub mode: DragMode,
    pub visited: [bool; INVENTORY_SIZE],
}

#[derive(Resource, Default)]
pub(crate) struct InventoryCursor {
    pub stack: Option<ItemStack>,
    pub hovered_slot: Option<usize>,
    pub drag: Option<DragState>,
}

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_sub_state::<InventoryState>()
            .init_resource::<InventoryCursor>()
            .add_systems(OnEnter(GameState::Game), ui::spawn_hotbar_hud)
            .add_systems(
                OnEnter(InventoryState::Open),
                (
                    ui::unlock_cursor,
                    ui::hide_game_hud,
                    ui::spawn_inventory_overlay,
                )
                    .chain(),
            )
            .add_systems(
                OnExit(InventoryState::Open),
                (ui::return_cursor_stack, ui::lock_cursor, ui::show_game_hud).chain(),
            )
            .add_systems(Update, toggle_inventory.run_if(in_state(GameState::Game)))
            .add_systems(
                Update,
                (handle_number_keys, handle_hotbar_scroll, ui::refresh_slots)
                    .run_if(in_state(GameState::Game)),
            )
            .add_systems(
                Update,
                ui::follow_cursor_stack.run_if(in_state(InventoryState::Open)),
            );
    }
}

fn toggle_inventory(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<InventoryState>>,
    mut next_state: ResMut<NextState<InventoryState>>,
) {
    if !keyboard.just_pressed(KeyCode::KeyE) {
        return;
    }

    next_state.set(match state.get() {
        InventoryState::Closed => InventoryState::Open,
        InventoryState::Open => InventoryState::Closed,
    });
}

fn pressed_hotbar_key(keyboard: &ButtonInput<KeyCode>) -> Option<usize> {
    [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
    .into_iter()
    .position(|key| keyboard.just_pressed(key))
}

fn handle_number_keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<InventoryState>>,
    cursor: Res<InventoryCursor>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
) {
    let Some(hotbar_index) = pressed_hotbar_key(&keyboard) else {
        return;
    };

    match state.get() {
        InventoryState::Closed => {
            inventory.set_selected_hotbar(hotbar_index);
        }
        InventoryState::Open => {
            let Some(slot_index) = cursor.hovered_slot else {
                return;
            };

            inventory.swap_slots(slot_index, hotbar_index);
        }
    }
}

fn handle_hotbar_scroll(
    scroll: Res<AccumulatedMouseScroll>,
    state: Res<State<InventoryState>>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
) {
    if *state.get() != InventoryState::Closed {
        return;
    }

    if scroll.delta.y == 0.0 {
        return;
    }

    let current = inventory.selected_hotbar();

    let next = if scroll.delta.y > 0.0 {
        (current + HOTBAR_SIZE - 1) % HOTBAR_SIZE
    } else {
        (current + 1) % HOTBAR_SIZE
    };

    inventory.set_selected_hotbar(next);
}
