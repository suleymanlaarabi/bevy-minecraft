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
        let owner = if index < crate::inventory::access::CONTAINER_START {
            Some(super::SlotOwner {
                entity: player,
                index,
            })
        } else {
            viewing.and_then(|viewing| {
                let offset = index - crate::inventory::access::CONTAINER_START;
                let entity = if offset < 27 {
                    viewing.0
                } else {
                    *partners.get(viewing.0).ok()?.first()?
                };
                Some(super::SlotOwner {
                    entity,
                    index: offset % 27,
                })
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
            Ref<crate::inventory::HotbarSelection>,
            Ref<crate::inventory::CraftingResult>,
        ),
        With<Player>,
    >,
    containers: Query<Ref<crate::block_behavior::ContainerInventory>>,
    new_views: Query<
        (),
        Or<(
            Added<super::SlotOwner>,
            Added<SlotIcon>,
            Added<SlotCount>,
            Added<EmptySlotLabel>,
        )>,
    >,
    block_icons: Res<BlockIcons>,
    mut icons: Query<(&ChildOf, &mut ImageNode, &mut Visibility), With<SlotIcon>>,
    mut counts: Query<
        (&ChildOf, &mut Text, Has<SlotCount>),
        Or<(With<SlotCount>, With<EmptySlotLabel>)>,
    >,
    owners: Query<(&InventorySlotView, &super::SlotOwner)>,
    mut slots: Query<(&InventorySlotView, &mut BorderColor)>,
) {
    let (player, inventory, selection, result) = inventory.into_inner();
    if !inventory.is_changed()
        && !selection.is_changed()
        && !result.is_changed()
        && !containers.iter().any(|inventory| inventory.is_changed())
        && new_views.is_empty()
    {
        return;
    }
    let slot = |owner: &super::SlotOwner| {
        if owner.entity == player {
            if owner.index == crate::inventory::CRAFT_RESULT {
                result.0
            } else {
                inventory.slot(owner.index)
            }
        } else {
            containers.get(owner.entity).ok()?.slot(owner.index)
        }
    };
    for (parent, mut node, mut visibility) in &mut icons {
        let Ok((_, owner)) = owners.get(parent.parent()) else {
            continue;
        };
        match slot(owner) {
            Some(stack) => {
                show_icon(&mut node, &block_icons, stack.item());
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }

    for (parent, mut text, count) in &mut counts {
        let Ok((view, owner)) = owners.get(parent.parent()) else {
            continue;
        };
        if count {
            let value = match slot(owner) {
                Some(stack) if stack.count() > 1 => stack.count().to_string(),
                _ => String::new(),
            };
            text.set_if_neq(Text(value));
        } else {
            let label = match slot(owner) {
                None => empty_slot_label(view.index),
                Some(_) => "",
            };
            // Compare first: the static label needs no allocation when unchanged.
            if text.0 != label {
                text.0 = label.into();
            }
        }
    }

    for (view, mut border) in &mut slots {
        let selected = view.index < HOTBAR_SIZE && view.index == selection.0;

        border.set_if_neq(BorderColor::all(if selected {
            SELECTED_BORDER
        } else {
            SLOT_BORDER
        }));
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
