use super::InventorySlotView;
use crate::{
    dropped_item::DroppedItemCommandsExt,
    game::GameState,
    inventory::{DragMode, DragState, INVENTORY_SIZE, InventoryCursor, PlayerInventory},
    player::Player,
};
use bevy::{picking::prelude::*, prelude::*};

pub(crate) fn return_cursor_stack(
    mut players: Query<(&mut PlayerInventory, &GlobalTransform), With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
    state: Res<State<GameState>>,
    mut commands: Commands,
) {
    // Leaving the session discards the cursor through `reset_inventory_cursor`.
    if *state.get() != GameState::Game {
        return;
    }
    cursor.hovered_slot = None;
    cursor.drag = None;
    let Some(stack) = cursor.stack.take() else {
        return;
    };
    let Ok((mut inventory, transform)) = players.single_mut() else {
        // The session may already have despawned its player.
        return;
    };
    if let Some(remaining) =
        PlayerInventory::edit(&mut inventory, |inventory| inventory.insert(stack))
    {
        commands.spawn_dropped_item(
            remaining,
            transform.translation() + Vec3::Y * 0.5,
            Vec3::Y * 1.2,
        );
    }
}

pub(super) fn slot_over(
    event: On<PointerOver>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    cursor.hovered_slot = Some(slot.index);
}

pub(super) fn slot_out(
    event: On<PointerOut>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    if cursor.hovered_slot == Some(slot.index) {
        cursor.hovered_slot = None;
    }
}

pub(super) fn slot_click(
    mut event: On<PointerClick>,
    slots: Query<&InventorySlotView>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    if cursor.drag.is_some() {
        return;
    }

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let index = slot.index;

    match event.button {
        PointerButton::Primary => {
            let shift =
                keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight);

            if shift && cursor.stack.is_none() {
                PlayerInventory::edit(&mut inventory, |inventory| inventory.quick_move(index));
                return;
            }

            if event.count >= 2 && cursor.stack.is_some() {
                PlayerInventory::edit(&mut inventory, |inventory| {
                    inventory.collect_matching(&mut cursor.stack)
                });
                return;
            }

            PlayerInventory::edit(&mut inventory, |inventory| {
                inventory.left_click(index, &mut cursor.stack)
            });
        }
        PointerButton::Secondary => {
            PlayerInventory::edit(&mut inventory, |inventory| {
                inventory.right_click(index, &mut cursor.stack)
            });
        }
        PointerButton::Middle => {}
    }
}

pub(super) fn slot_drag_start(
    mut event: On<PointerDragStart>,
    slots: Query<&InventorySlotView>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let had_stack_before_drag = cursor.stack.is_some();

    let mode = match event.button {
        PointerButton::Primary => DragMode::Even,
        PointerButton::Secondary => DragMode::OneEach,
        PointerButton::Middle => return,
    };

    if !had_stack_before_drag {
        PlayerInventory::edit(&mut inventory, |inventory| {
            if event.button == PointerButton::Primary {
                inventory.left_click(slot.index, &mut cursor.stack);
            } else {
                inventory.right_click(slot.index, &mut cursor.stack);
            }
        });
    }

    if cursor.stack.is_none() {
        return;
    }

    let mut visited = [false; INVENTORY_SIZE];

    if had_stack_before_drag {
        visited[slot.index] = true;
    }

    cursor.drag = Some(DragState {
        source_entity: event.entity,
        mode,
        visited,
    });
}

pub(super) fn slot_drag_enter(
    mut event: On<PointerDragEnter>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let Some(drag) = cursor.drag.as_mut() else {
        return;
    };

    if event.dragged != drag.source_entity {
        return;
    }

    drag.visited[slot.index] = true;
}

pub(super) fn slot_drag_over(
    mut event: On<PointerDragOver>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let Some(drag) = cursor.drag.as_mut() else {
        return;
    };

    if event.dragged != drag.source_entity {
        return;
    }

    drag.visited[slot.index] = true;
}

pub(super) fn slot_drag_end(
    mut event: On<PointerDragEnd>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Some(drag) = cursor.drag.take() else {
        return;
    };

    if drag.source_entity != event.entity {
        cursor.drag = Some(drag);
        return;
    }

    PlayerInventory::edit(&mut inventory, |inventory| {
        inventory.distribute_drag(&mut cursor.stack, &drag.visited, drag.mode)
    });
}
