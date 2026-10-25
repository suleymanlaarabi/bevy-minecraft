use super::{
    CursorStackCount, CursorStackView, HotbarHud, InventorySlotView, SELECTED_BORDER, SLOT_BORDER,
    SlotCount, SlotIcon,
};
use crate::{
    inventory::{HOTBAR_SIZE, InventoryCursor, ItemKind, PlayerInventory, icons::BlockIcons},
    player::{Crosshair, Player},
};
use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

pub(crate) fn unlock_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut cursor in &mut cursors {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

pub(crate) fn lock_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
}

pub(crate) fn hide_game_hud(
    mut hotbar: Query<&mut Visibility, With<HotbarHud>>,
    mut crosshair: Query<&mut Visibility, (With<Crosshair>, Without<HotbarHud>)>,
) {
    for mut visibility in &mut hotbar {
        *visibility = Visibility::Hidden;
    }

    for mut visibility in &mut crosshair {
        *visibility = Visibility::Hidden;
    }
}

pub(crate) fn show_game_hud(
    mut hotbar: Query<&mut Visibility, With<HotbarHud>>,
    mut crosshair: Query<&mut Visibility, (With<Crosshair>, Without<HotbarHud>)>,
) {
    for mut visibility in &mut hotbar {
        *visibility = Visibility::Inherited;
    }

    for mut visibility in &mut crosshair {
        *visibility = Visibility::Inherited;
    }
}

/// Points `node` at the icon of `item`, leaving it untouched when already shown.
fn show_icon(node: &mut Mut<ImageNode>, icons: &BlockIcons, item: ItemKind) {
    let atlas = icons.atlas(item);
    let shown = node.image == *icons.image()
        && node
            .texture_atlas
            .as_ref()
            .is_some_and(|current| current.index == atlas.index);
    if !shown {
        node.image = icons.image().clone();
        node.texture_atlas = Some(atlas);
    }
}

pub(crate) fn refresh_slots(
    inventory: Single<Ref<PlayerInventory>, With<Player>>,
    new_views: Query<(), Added<InventorySlotView>>,
    block_icons: Res<BlockIcons>,
    mut icons: Query<(&SlotIcon, &mut ImageNode, &mut Visibility)>,
    mut counts: Query<(&SlotCount, &mut Text)>,
    mut slots: Query<(&InventorySlotView, &mut BorderColor)>,
) {
    if !inventory.is_changed() && new_views.is_empty() {
        return;
    }
    for (view, mut node, mut visibility) in &mut icons {
        match inventory.slot(view.index) {
            Some(stack) => {
                show_icon(&mut node, &block_icons, stack.item());
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }

    for (view, mut text) in &mut counts {
        let value = match inventory.slot(view.index) {
            Some(stack) if stack.count() > 1 => stack.count().to_string(),
            _ => String::new(),
        };
        text.set_if_neq(Text(value));
    }

    for (view, mut border) in &mut slots {
        let selected = view.index < HOTBAR_SIZE && view.index == inventory.selected_hotbar();

        border.set_if_neq(BorderColor::all(if selected {
            SELECTED_BORDER
        } else {
            SLOT_BORDER
        }));
    }
}

pub(crate) fn follow_cursor_stack(
    window: Single<&Window, With<PrimaryWindow>>,
    cursor: Res<InventoryCursor>,
    block_icons: Res<BlockIcons>,
    view: Single<(&mut Node, &mut ImageNode, &mut Visibility), With<CursorStackView>>,
    mut count: Single<&mut Text, With<CursorStackCount>>,
) {
    let (mut node, mut image, mut visibility) = view.into_inner();

    let Some(stack) = cursor.stack else {
        visibility.set_if_neq(Visibility::Hidden);
        if !count.0.is_empty() {
            count.0.clear();
        }
        return;
    };

    let Some(position) = window.cursor_position() else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };

    visibility.set_if_neq(Visibility::Inherited);
    let left = px(position.x + 12.0);
    let top = px(position.y + 12.0);
    if node.left != left || node.top != top {
        node.left = left;
        node.top = top;
    }
    show_icon(&mut image, &block_icons, stack.item());
    let text = if stack.count() > 1 {
        stack.count().to_string()
    } else {
        String::new()
    };
    count.set_if_neq(Text(text));
}
