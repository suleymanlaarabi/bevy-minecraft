//! Inventory view components and the systems exposed to InventoryPlugin.

mod interactions;
mod scenes;
mod sync;

use bevy::prelude::*;

pub(crate) use interactions::return_cursor_stack;
pub(crate) use scenes::{spawn_hotbar_hud, spawn_inventory_overlay};
pub(crate) use sync::{
    follow_cursor_stack, hide_game_hud, lock_cursor, refresh_slots, show_game_hud, unlock_cursor,
};

const SLOT_SIZE: f32 = 48.0;
const SLOT_GAP: f32 = 4.0;
const ICON_SIZE: f32 = 32.0;

const SLOT_BG: Color = Color::srgba(0.08, 0.08, 0.09, 0.94);
const SLOT_BORDER: Color = Color::srgba(0.35, 0.35, 0.38, 1.0);
const SELECTED_BORDER: Color = Color::WHITE;
const PANEL_BG: Color = Color::srgba(0.10, 0.10, 0.11, 0.97);

#[derive(Component, Clone, Default)]
pub(crate) struct HotbarHud;

#[derive(Component, Clone, Default)]
struct InventoryOverlay;

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct InventorySlotView {
    index: usize,
}

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct SlotIcon {
    index: usize,
}

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct SlotCount {
    index: usize,
}

#[derive(Component, Clone, Default)]
pub(crate) struct CursorStackView;

#[derive(Component, Clone, Default)]
pub(crate) struct CursorStackCount;

#[cfg(test)]
mod tests;
