use super::InventorySlotView;
use crate::{
    dropped_item::DroppedItemCommandsExt,
    game::GameState,
    inventory::{DragMode, InventoryCursor, PlayerInventory},
    player::Player,
};
use bevy::{picking::prelude::*, prelude::*};

pub(crate) fn return_cursor_stack(
    mut players: Query<
        (
            &mut PlayerInventory,
            &mut crate::inventory::CraftingGrid,
            &GlobalTransform,
        ),
        With<Player>,
    >,
    mut cursor: ResMut<InventoryCursor>,
    mut actions: ResMut<Messages<crate::inventory::access::InventoryAction>>,
    state: Res<State<GameState>>,
    mut commands: Commands,
) {
    actions.clear();
    // Leaving the session discards the cursor through `reset_inventory_cursor`.
    if *state.get() != GameState::Game {
        return;
    }
    cursor.hovered_slot = None;
    cursor.drag = None;
    let held = cursor.stack.take();
    let Ok((mut inventory, mut grid, transform)) = players.single_mut() else {
        return;
    };
    PlayerInventory::edit(&mut inventory, |inventory| {
        let inputs = std::mem::take(&mut grid.slots);
        grid.side = 2;
        for stack in held.into_iter().chain(inputs.into_iter().flatten()) {
            if let Some(remaining) = inventory.insert(stack) {
                commands.spawn_dropped_item(
                    remaining,
                    transform.translation() + Vec3::Y * 0.5,
                    Vec3::Y * 1.2,
                );
            }
        }
    });
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

fn click_action(
    index: usize,
    right: bool,
    double: bool,
    keyboard: &ButtonInput<KeyCode>,
) -> crate::inventory::access::InventoryAction {
    crate::inventory::access::InventoryAction::Click {
        index,
        right,
        double,
        shift: keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight),
    }
}
pub(super) fn slot_press(
    mut event: On<PointerPress>,
    slots: Query<&InventorySlotView>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut actions: MessageWriter<crate::inventory::access::InventoryAction>,
) {
    if event.button != PointerButton::Secondary {
        return;
    }
    event.propagate(false);
    if let Ok(slot) = slots.get(event.entity) {
        actions.write(click_action(slot.index, true, false, &keyboard));
    }
}
pub(super) fn slot_click(
    mut event: On<PointerClick>,
    slots: Query<&InventorySlotView>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut actions: MessageWriter<crate::inventory::access::InventoryAction>,
) {
    event.propagate(false);
    if event.button == PointerButton::Primary
        && let Ok(slot) = slots.get(event.entity)
    {
        actions.write(click_action(slot.index, false, event.count >= 2, &keyboard));
    }
}
pub(super) fn slot_drag_start(
    mut event: On<PointerDragStart>,
    slots: Query<&InventorySlotView>,
    mut actions: MessageWriter<crate::inventory::access::InventoryAction>,
) {
    event.propagate(false);
    let mode = match event.button {
        PointerButton::Primary => DragMode::Even,
        PointerButton::Secondary => DragMode::OneEach,
        PointerButton::Middle => return,
    };
    if let Ok(slot) = slots.get(event.entity) {
        actions.write(crate::inventory::access::InventoryAction::DragStart {
            index: slot.index,
            source: event.entity,
            mode,
        });
    }
}
pub(super) fn slot_drag_enter(
    mut event: On<PointerDragEnter>,
    slots: Query<&InventorySlotView>,
    mut actions: MessageWriter<crate::inventory::access::InventoryAction>,
) {
    event.propagate(false);
    if let Ok(slot) = slots.get(event.entity) {
        actions.write(crate::inventory::access::InventoryAction::DragVisit {
            index: slot.index,
            source: event.dragged,
        });
    }
}
pub(super) fn slot_drag_over(
    mut event: On<PointerDragOver>,
    slots: Query<&InventorySlotView>,
    mut actions: MessageWriter<crate::inventory::access::InventoryAction>,
) {
    event.propagate(false);
    if let Ok(slot) = slots.get(event.entity) {
        actions.write(crate::inventory::access::InventoryAction::DragVisit {
            index: slot.index,
            source: event.dragged,
        });
    }
}
pub(super) fn slot_drag_end(
    mut event: On<PointerDragEnd>,
    mut actions: MessageWriter<crate::inventory::access::InventoryAction>,
) {
    event.propagate(false);
    actions.write(crate::inventory::access::InventoryAction::DragEnd(
        event.entity,
    ));
}
