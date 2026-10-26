use super::{
    CursorStackCount, CursorStackView, EmptySlotLabel, HotbarHud, ICON_SIZE, InventorySlotView,
    SELECTED_BORDER, SLOT_BORDER, SlotCount, SlotIcon, empty_slot_label,
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
    if let Some(image) = icons.item_image(item) {
        let atlas = icons.item_atlas(item);
        if node.image != *image
            || node.texture_atlas.as_ref().map(|a| a.index) != atlas.as_ref().map(|a| a.index)
        {
            node.image = image.clone();
            node.texture_atlas = atlas;
        }
        return;
    }
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

pub(crate) fn bind_slots(
    mut commands: Commands,
    player: Single<
        (
            Entity,
            Option<&crate::block_behavior::containers::ViewingContainer>,
        ),
        With<Player>,
    >,
    partners: Query<&crate::block_behavior::containers::ChestPartners>,
    views: Query<(Entity, &InventorySlotView), Added<InventorySlotView>>,
) {
    let (player, viewing) = *player;
    for (entity, view) in &views {
        let index = view.index;
        let owner = if index < crate::inventory::SLOT_COUNT {
            Some(super::SlotOwner::Inventory(player, index))
        } else if index < crate::inventory::CRAFT_RESULT {
            Some(super::SlotOwner::Crafting(
                player,
                index - crate::inventory::CRAFT_START,
            ))
        } else if index == crate::inventory::CRAFT_RESULT {
            Some(super::SlotOwner::Result(player))
        } else {
            viewing.and_then(|viewing| {
                let offset = index - crate::inventory::access::CONTAINER_START;
                let entity = if offset < 27 {
                    viewing.0
                } else {
                    *partners.get(viewing.0).ok()?.first()?
                };
                Some(super::SlotOwner::Container(entity, offset % 27))
            })
        };
        if let Some(owner) = owner {
            commands.entity(entity).insert(owner);
        }
    }
}
pub(crate) fn refresh_slots(
    inventory: Single<
        (
            Entity,
            Ref<PlayerInventory>,
            Ref<crate::inventory::CraftingGrid>,
            Ref<crate::inventory::HotbarSelection>,
            Ref<crate::inventory::CraftingResult>,
            Option<&crate::block_behavior::containers::ViewingContainer>,
        ),
        With<Player>,
    >,
    containers: Query<Ref<crate::block_behavior::ContainerInventory>>,
    partners: Query<&crate::block_behavior::containers::ChestPartners>,
    new_views: Query<(), Added<super::SlotOwner>>,
    block_icons: Res<BlockIcons>,
    mut icons: Query<(&mut ImageNode, &mut Visibility), With<SlotIcon>>,
    mut durability: Query<
        (&mut Node, &mut Visibility),
        (With<super::SlotDurability>, Without<SlotIcon>),
    >,
    mut counts: Query<(&mut Text, Has<SlotCount>), Or<(With<SlotCount>, With<EmptySlotLabel>)>>,
    mut views: Query<(
        &InventorySlotView,
        &super::SlotOwner,
        &Children,
        &mut BorderColor,
        &mut super::SlotDisplay,
    )>,
) {
    let (player, inventory, grid, selection, result, viewing) = inventory.into_inner();
    let container_changed = |entity| {
        containers
            .get(entity)
            .is_ok_and(|inventory| inventory.is_changed())
    };
    let visible_containers_changed = viewing.is_some_and(|viewing| {
        container_changed(viewing.0)
            || partners
                .get(viewing.0)
                .is_ok_and(|partners| partners.iter().any(|entity| container_changed(entity)))
    });
    if !inventory.is_changed()
        && !grid.is_changed()
        && !selection.is_changed()
        && !result.is_changed()
        && !visible_containers_changed
        && new_views.is_empty()
    {
        return;
    }
    for (view, owner, children, mut border, mut display) in &mut views {
        border.set_if_neq(BorderColor::all(
            if view.index < HOTBAR_SIZE && view.index == selection.0 {
                SELECTED_BORDER
            } else {
                SLOT_BORDER
            },
        ));
        let stack = match *owner {
            super::SlotOwner::Inventory(entity, index) if entity == player => inventory.slot(index),
            super::SlotOwner::Crafting(entity, index) if entity == player => {
                grid.slots.get(index).copied().flatten()
            }
            super::SlotOwner::Result(entity) if entity == player => result.0,
            super::SlotOwner::Container(entity, index) => containers
                .get(entity)
                .ok()
                .and_then(|inventory| inventory.slot(index)),
            _ => None,
        };
        if display.0 == Some(stack) {
            continue;
        }
        display.0 = Some(stack);
        for &child in children {
            if let Ok((mut node, mut visibility)) = icons.get_mut(child) {
                if let Some(stack) = stack {
                    show_icon(&mut node, &block_icons, stack.item());
                    visibility.set_if_neq(Visibility::Inherited);
                } else {
                    visibility.set_if_neq(Visibility::Hidden);
                }
            }
            if let Ok((mut text, count)) = counts.get_mut(child) {
                if count {
                    text.set_if_neq(Text(
                        stack
                            .filter(|stack| stack.count() > 1)
                            .map_or_else(String::new, |stack| stack.count().to_string()),
                    ));
                } else {
                    let label = if stack.is_some() {
                        ""
                    } else {
                        empty_slot_label(view.index)
                    };
                    if text.0 != label {
                        text.0 = label.into();
                    }
                }
            }
            if let Ok((mut node, mut visibility)) = durability.get_mut(child) {
                if let Some(stack) = stack
                    && stack.damage() > 0
                    && stack.item().max_damage() > 0
                {
                    let fraction = 1.0 - stack.damage() as f32 / stack.item().max_damage() as f32;
                    node.width = px((super::SLOT_SIZE - 8.0) * fraction);
                    visibility.set_if_neq(Visibility::Inherited);
                } else {
                    visibility.set_if_neq(Visibility::Hidden);
                }
            }
        }
    }
}

pub(crate) fn follow_cursor_stack(
    window: Single<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
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
    // Window coordinates already account for DPI; only the UI scale remains.
    let position = position / ui_scale.0 - Vec2::splat(ICON_SIZE * 0.5);
    let left = px(position.x);
    let top = px(position.y);
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
