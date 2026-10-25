use crate::{
    inventory::{
        CRAFT_RESULT, HOTBAR_SIZE, InventoryCursor, InventoryState, OFFHAND_SLOT, PlayerInventory,
    },
    player::Player,
};
use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*};

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

pub(super) fn handle_number_keys(
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
            PlayerInventory::edit(&mut inventory, |inventory| {
                inventory.set_selected_hotbar(hotbar_index)
            });
        }
        InventoryState::Open => {
            let Some(slot_index) = cursor.hovered_slot else {
                return;
            };

            PlayerInventory::edit(&mut inventory, |inventory| {
                if slot_index == CRAFT_RESULT {
                    inventory.craft_to_hotbar(hotbar_index);
                } else {
                    inventory.swap_slots(slot_index, hotbar_index);
                }
            });
        }
    }
}

pub(super) fn handle_hotbar_scroll(
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

pub(super) fn handle_swap_offhand(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<InventoryState>>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
) {
    if *state.get() == InventoryState::Closed && keyboard.just_pressed(KeyCode::KeyF) {
        PlayerInventory::edit(&mut inventory, |inventory| {
            inventory.swap_slots(inventory.selected_hotbar(), OFFHAND_SLOT)
        });
    }
}
