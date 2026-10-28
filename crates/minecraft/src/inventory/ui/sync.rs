use super::{
    CursorStackView, EmptySlotLabel, ICON_SIZE, InventorySlotView, SELECTED_BORDER, SLOT_BORDER,
    SlotCount, SlotIcon, empty_slot_label,
};
use crate::{
    inventory::{HOTBAR_SIZE, InventoryCursor, ItemKind, PlayerInventory, icons::BlockIcons},
    player::Player,
};
use bevy::{prelude::*, window::PrimaryWindow};

/// Points `node` at the icon of `item`, leaving it untouched when already shown.
fn show_icon(node: &mut Mut<ImageNode>, icons: &BlockIcons, item: ItemKind) {
    let (image, atlas) = if let Some(image) = icons.item_image(item) {
        (image, icons.item_atlas(item))
    } else {
        (icons.image(), Some(icons.atlas(item)))
    };
    if node.image != *image || node.texture_atlas != atlas {
        node.image = image.clone();
        node.texture_atlas = atlas;
    }
}

fn stack_count(stack: Option<crate::inventory::ItemStack>) -> Text {
    Text(
        stack
            .filter(|stack| stack.count() > 1)
            .map_or_else(String::new, |stack| stack.count().to_string()),
    )
}

pub(crate) fn bind_slots(
    mut commands: Commands,
    player: Single<
        (
            Entity,
            Option<&crate::inventory::InventorySession>,
            &hollow_features::motion::FixedPosition,
        ),
        With<Player>,
    >,
    access: crate::inventory::session::StationAccess,
    views: Query<(Entity, &InventorySlotView), Added<InventorySlotView>>,
) {
    if views.is_empty() {
        return;
    }
    let (player, viewing, position) = *player;
    let Some(layout) = access.layout(viewing, position.current.as_vec3()) else {
        return;
    };
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
            let offset = index - crate::inventory::access::CONTAINER_START;
            let size = crate::block_behavior::containers::CONTAINER_SIZE;
            layout
                .containers
                .get(offset / size)
                .copied()
                .flatten()
                .map(|owner| super::SlotOwner::Container(owner, offset % size))
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
            Option<&crate::inventory::InventorySession>,
        ),
        With<Player>,
    >,
    containers: Query<Ref<crate::block_behavior::ContainerInventory>>,
    partners: Query<&crate::block_behavior::containers::ChestPartners>,
    new_views: Query<(), Added<super::SlotOwner>>,
    mut views: Query<(
        &InventorySlotView,
        &super::SlotOwner,
        &mut BorderColor,
        &mut super::StackView,
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
    for (view, owner, mut border, mut display) in &mut views {
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
        display.set_if_neq(super::StackView(stack));
    }
}

/// Ownership and pointer movement only publish stack values; this pass owns rendering.
pub(crate) fn sync_stack_views(
    block_icons: Res<BlockIcons>,
    views: Query<(Ref<super::StackView>, &Children, Option<&InventorySlotView>)>,
    mut icons: Query<(&mut ImageNode, &mut Visibility), With<SlotIcon>>,
    mut durability: Query<
        (&mut Node, &mut Visibility),
        (With<super::SlotDurability>, Without<SlotIcon>),
    >,
    mut counts: Query<(&mut Text, Has<SlotCount>), Or<(With<SlotCount>, With<EmptySlotLabel>)>>,
) {
    for (view, children, slot) in &views {
        if !view.is_changed() && !block_icons.is_changed() {
            continue;
        }
        let stack = view.0;
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
                    text.set_if_neq(stack_count(stack));
                } else {
                    let label = if stack.is_some() {
                        ""
                    } else {
                        slot.map_or("", |slot| empty_slot_label(slot.index))
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
    view: Single<(&mut Node, &mut Visibility, &mut super::StackView), With<CursorStackView>>,
) {
    let (mut node, mut visibility, mut view) = view.into_inner();
    view.set_if_neq(super::StackView(cursor.stack));
    let position = window.cursor_position().filter(|_| cursor.stack.is_some());
    visibility.set_if_neq(if position.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    let Some(position) = position else {
        return;
    };
    // Window coordinates already account for DPI; only the UI scale remains.
    let position = position / ui_scale.0 - Vec2::splat(ICON_SIZE * 0.5);
    let left = px(position.x);
    let top = px(position.y);
    if node.left != left || node.top != top {
        node.left = left;
        node.top = top;
    }
}
