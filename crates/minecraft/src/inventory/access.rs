//! Input is ordered before mutations; each action batch borrows its owners once.
use super::{
    CRAFT_RESULT, DragMode, DragState, INVENTORY_SIZE, InventoryCursor, ItemKind, ItemStack,
    PlayerInventory, SLOT_COUNT, slots::Slots,
};
use crate::{
    block_behavior::containers::{
        CONTAINER_SIZE, ContainerInventory, ContainerTopology, ViewingContainer,
    },
    player::Player,
};
use bevy::prelude::*;
use hollow_features::motion::FixedPosition;
use std::collections::VecDeque;

pub(crate) const CONTAINER_START: usize = CRAFT_RESULT + 1;
pub(crate) const MAX_VIEW_SLOTS: usize = CONTAINER_START + CONTAINER_SIZE * 2;
#[derive(Clone, Copy, Debug)]
pub(crate) enum InventoryAction {
    SwapHotbar(usize, usize),
    Click {
        index: usize,
        right: bool,
        shift: bool,
        double: bool,
    },
    DragStart {
        index: usize,
        source: Entity,
        mode: DragMode,
    },
    DragVisit {
        index: usize,
        source: Entity,
    },
    DragEnd(Entity),
}
#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct InventoryActions(VecDeque<InventoryAction>);

pub(crate) fn apply_actions(
    mut actions: ResMut<InventoryActions>,
    mut players: Query<
        (
            &mut PlayerInventory,
            &mut super::CraftingGrid,
            Option<&crate::block_behavior::workbench::ViewingWorkbench>,
            &FixedPosition,
            Option<&ViewingContainer>,
        ),
        With<Player>,
    >,
    topology: ContainerTopology,
    mut containers: Query<&mut ContainerInventory>,
    mut cursor: ResMut<InventoryCursor>,
    workbenches: crate::block_behavior::workbench::WorkbenchAccess,
) {
    if actions.is_empty() {
        return;
    }
    // Dropping the drain also discards actions when their session is invalid.
    let actions = actions.drain(..);
    let Ok((mut inventory, mut grid, workbench, motion, viewing)) = players.single_mut() else {
        return;
    };
    if let Some(session) = workbench
        && !workbenches.valid(session.0, motion.current.as_vec3())
    {
        return;
    }
    if (workbench.is_some() && viewing.is_some())
        || grid.side != if workbench.is_some() { 3 } else { 2 }
    {
        return;
    }
    let mut owners = if let Some(viewing) = viewing {
        let Some(owners) = topology.owners(viewing.0, motion.current.as_vec3()) else {
            return;
        };
        let first = owners[0].expect("Canonical owner");
        if let Some(second) = owners[1] {
            let Ok([first, second]) = containers.get_many_mut([first, second]) else {
                return;
            };
            [Some(first), Some(second)]
        } else {
            let Ok(first) = containers.get_mut(first) else {
                return;
            };
            [Some(first), None]
        }
    } else {
        [None, None]
    };
    for action in actions {
        let before = *grid;
        PlayerInventory::edit(&mut inventory, |inventory| {
            let mut slots = ScreenSlots {
                inventory,
                grid: grid.bypass_change_detection(),
                containers: &mut owners,
            };
            match action {
                InventoryAction::SwapHotbar(index, hotbar) => {
                    if index == CRAFT_RESULT && !slots.container_open() {
                        super::crafting::craft(
                            slots.inventory,
                            slots.grid,
                            &mut cursor.stack,
                            super::crafting::CraftDestination::Hotbar(hotbar),
                        );
                    } else {
                        super::slots::swap(&mut slots, index, hotbar);
                    }
                }
                InventoryAction::Click {
                    index,
                    right,
                    shift,
                    double,
                } => {
                    if cursor.drag.is_some() {
                        return;
                    }
                    if index == CRAFT_RESULT {
                        if !slots.container_open() {
                            super::crafting::craft(
                                slots.inventory,
                                slots.grid,
                                &mut cursor.stack,
                                if shift {
                                    super::crafting::CraftDestination::Inventory
                                } else {
                                    super::crafting::CraftDestination::Cursor
                                },
                            );
                        }
                    } else if shift && !right && cursor.stack.is_none() {
                        slots.quick_move(index);
                    } else if double && cursor.stack.is_some() {
                        super::slots::collect(&mut slots, &mut cursor.stack);
                    } else {
                        super::slots::click(&mut slots, index, &mut cursor.stack, right);
                    }
                }
                InventoryAction::DragStart {
                    index,
                    source,
                    mode,
                } => {
                    if index == CRAFT_RESULT || index >= slots.len() {
                        return;
                    }
                    let had_stack = cursor.stack.is_some();
                    if !had_stack && mode == DragMode::Even {
                        super::slots::click(&mut slots, index, &mut cursor.stack, false);
                    }
                    if cursor.stack.is_none() {
                        return;
                    }
                    let mut visited = [false; MAX_VIEW_SLOTS];
                    visited[index] = had_stack || mode == DragMode::OneEach;
                    cursor.drag = Some(DragState {
                        source_entity: source,
                        mode,
                        visited,
                    });
                }
                InventoryAction::DragVisit { index, source } => {
                    let Some(drag) = cursor.drag.as_mut() else {
                        return;
                    };
                    if source != drag.source_entity
                        || index >= drag.visited.len()
                        || drag.visited[index]
                    {
                        return;
                    }
                    drag.visited[index] = true;
                    if drag.mode == DragMode::OneEach {
                        super::slots::click(&mut slots, index, &mut cursor.stack, true);
                    }
                }
                InventoryAction::DragEnd(source) => {
                    if cursor
                        .drag
                        .as_ref()
                        .is_none_or(|drag| drag.source_entity != source)
                    {
                        return;
                    }
                    let drag = cursor.drag.take().expect("Active drag");
                    if drag.mode == DragMode::Even {
                        super::slots::distribute(
                            &mut slots,
                            &mut cursor.stack,
                            &drag.visited,
                            drag.mode,
                        );
                    }
                }
            }
        });
        if *grid != before {
            grid.set_changed();
        }
    }
}

/// The only borrowed adapter crossing player and chest slot owners.
struct ScreenSlots<'a, 'w> {
    inventory: &'a mut PlayerInventory,
    grid: &'a mut super::CraftingGrid,
    containers: &'a mut [Option<Mut<'w, ContainerInventory>>; 2],
}
impl ScreenSlots<'_, '_> {
    fn container_open(&self) -> bool {
        self.containers[0].is_some()
    }
    fn quick_move(&mut self, index: usize) {
        if self.container_open() {
            let targets = if index < INVENTORY_SIZE {
                CONTAINER_START..self.len()
            } else {
                0..INVENTORY_SIZE
            };
            super::slots::transfer(self, index, targets);
        } else if (super::CRAFT_START..CRAFT_RESULT).contains(&index) {
            super::slots::transfer(self, index, 0..INVENTORY_SIZE);
        } else {
            self.inventory.quick_move(index);
        }
    }
}
impl Slots for ScreenSlots<'_, '_> {
    type Stack = ItemStack;
    fn len(&self) -> usize {
        if self.container_open() {
            CONTAINER_START + self.containers.iter().flatten().count() * CONTAINER_SIZE
        } else {
            super::CRAFT_START + self.grid.active_len()
        }
    }
    fn get(&self, index: usize) -> Option<ItemStack> {
        if index < SLOT_COUNT {
            self.inventory.slot(index)
        } else if index < CRAFT_RESULT {
            self.grid
                .slots
                .as_slice()
                .get(index - super::CRAFT_START)
                .copied()
                .flatten()
        } else {
            let offset = index.checked_sub(CONTAINER_START)?;
            self.containers
                .get(offset / CONTAINER_SIZE)?
                .as_ref()?
                .slot(offset % CONTAINER_SIZE)
        }
    }
    fn set(&mut self, index: usize, stack: Option<ItemStack>) {
        if index < SLOT_COUNT {
            Slots::set(self.inventory, index, stack);
        } else if (super::CRAFT_START..super::CRAFT_START + self.grid.active_len()).contains(&index)
        {
            self.grid.slots[index - super::CRAFT_START] = stack;
        } else if let Some(offset) = index.checked_sub(CONTAINER_START)
            && let Some(Some(inventory)) = self.containers.get_mut(offset / CONTAINER_SIZE)
            && inventory
                .bypass_change_detection()
                .set(offset % CONTAINER_SIZE, stack)
        {
            inventory.set_changed();
        }
    }
    fn writable(&self, index: usize) -> bool {
        if self.container_open() {
            index < INVENTORY_SIZE || (CONTAINER_START..self.len()).contains(&index)
        } else {
            index < super::CRAFT_START + self.grid.active_len()
        }
    }
    fn capacity(&self, index: usize, item: ItemKind) -> u8 {
        if !self.writable(index) {
            0
        } else if index < SLOT_COUNT {
            Slots::capacity(self.inventory, index, item)
        } else {
            item.max_stack_size()
        }
    }
    fn collectible(&self, index: usize) -> bool {
        index < INVENTORY_SIZE || (CONTAINER_START..self.len()).contains(&index)
    }
}
