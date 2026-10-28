//! Inventory view components and the systems exposed to InventoryPlugin.

mod interactions;
mod scenes;
mod sync;

use bevy::prelude::*;

pub(crate) use interactions::return_cursor_stack;
pub(crate) use scenes::{spawn_hotbar_hud, spawn_inventory_overlay};
pub(crate) use sync::{bind_slots, follow_cursor_stack, refresh_slots, sync_stack_views};

const SLOT_SIZE: f32 = 48.0;
const SLOT_GAP: f32 = 4.0;
const ICON_SIZE: f32 = 32.0;

const SLOT_BG: Color = Color::srgba(0.08, 0.08, 0.09, 0.94);
const SLOT_BORDER: Color = Color::srgba(0.35, 0.35, 0.38, 1.0);
const SELECTED_BORDER: Color = Color::WHITE;
const PANEL_BG: Color = Color::srgba(0.10, 0.10, 0.11, 0.97);

#[derive(Component, Clone, Default)]
#[require(hollow_features::cursor::FreeCursor)]
struct InventoryOverlay;

#[derive(Component, Clone, Copy, Default)]
#[require(StackView)]
pub(crate) struct InventorySlotView {
    index: usize,
}

/// Presentation input shared by slot owners, recipe results, and the cursor.
#[derive(Component, Default, Clone, Copy, PartialEq)]
pub(crate) struct StackView(pub Option<crate::inventory::ItemStack>);

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct SlotIcon;

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct SlotCount;

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct EmptySlotLabel;

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct SlotDurability;

fn empty_slot_label(index: usize) -> &'static str {
    use crate::inventory::{ARMOR_START, OFFHAND_SLOT};
    match index {
        index if index == ARMOR_START => "Head",
        index if index == ARMOR_START + 1 => "Chest",
        index if index == ARMOR_START + 2 => "Legs",
        index if index == ARMOR_START + 3 => "Feet",
        OFFHAND_SLOT => "Hand",
        _ => "",
    }
}

#[derive(Component, Clone, Default)]
#[require(StackView)]
pub(crate) struct CursorStackView;

/// A view addresses its ECS owner directly; no mirror inventory is kept in UI.
#[derive(Component, Clone, Copy)]
pub(super) enum SlotOwner {
    Inventory(Entity, usize),
    Crafting(Entity, usize),
    Result(Entity),
    Container(Entity, usize),
}
