use bevy::prelude::*;

use crate::voxel::VoxelKind;

pub const HOTBAR_SIZE: usize = 9;
pub const MAIN_INVENTORY_SIZE: usize = 27;
pub const INVENTORY_SIZE: usize = HOTBAR_SIZE + MAIN_INVENTORY_SIZE;
pub const BLOCK_STACK_SIZE: u8 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Block(VoxelKind),
}

impl ItemKind {
    pub const fn max_stack_size(self) -> u8 {
        match self {
            Self::Block(_) => BLOCK_STACK_SIZE,
        }
    }

    pub const fn is_valid(self) -> bool {
        !matches!(self, Self::Block(VoxelKind::Air | VoxelKind::Water))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    item: ItemKind,
    count: u8,
}

impl ItemStack {
    pub fn new(item: ItemKind, count: u8) -> Self {
        assert!(item.is_valid());
        assert!(count > 0);
        assert!(count <= item.max_stack_size());

        Self { item, count }
    }

    pub const fn item(self) -> ItemKind {
        self.item
    }

    pub const fn count(self) -> u8 {
        self.count
    }

    pub const fn max_stack_size(self) -> u8 {
        self.item.max_stack_size()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragMode {
    Even,
    OneEach,
}

#[derive(Component, Clone, Debug)]
pub struct PlayerInventory {
    slots: [Option<ItemStack>; INVENTORY_SIZE],
    selected_hotbar: usize,
}

impl Default for PlayerInventory {
    fn default() -> Self {
        let mut slots = [None; INVENTORY_SIZE];

        slots[0] = Some(ItemStack::new(ItemKind::Block(VoxelKind::Dirt), 2));

        Self {
            slots,
            selected_hotbar: 0,
        }
    }
}

impl PlayerInventory {
    pub fn slot(&self, index: usize) -> Option<ItemStack> {
        self.slots.get(index).copied().flatten()
    }

    pub const fn selected_hotbar(&self) -> usize {
        self.selected_hotbar
    }

    pub fn selected_stack(&self) -> Option<ItemStack> {
        self.slots[self.selected_hotbar]
    }

    pub fn set_selected_hotbar(&mut self, index: usize) {
        self.selected_hotbar = index.min(HOTBAR_SIZE - 1);
    }

    pub fn swap_slots(&mut self, a: usize, b: usize) {
        if a >= INVENTORY_SIZE || b >= INVENTORY_SIZE || a == b {
            return;
        }

        self.slots.swap(a, b);
    }

    pub fn insert(&mut self, stack: ItemStack) -> Option<ItemStack> {
        let mut remaining = stack;

        for existing in self.slots.iter_mut().flatten() {
            if existing.item != remaining.item {
                continue;
            }

            let capacity = existing.max_stack_size() - existing.count;
            let moved = capacity.min(remaining.count);

            existing.count += moved;
            remaining.count -= moved;

            if remaining.count == 0 {
                return None;
            }
        }

        if let Some(empty) = self.slots.iter_mut().find(|slot| slot.is_none()) {
            *empty = Some(remaining);
            return None;
        }

        Some(remaining)
    }

    pub fn quick_move(&mut self, index: usize) {
        if index >= INVENTORY_SIZE {
            return;
        }

        let Some(stack) = self.slots[index].take() else {
            return;
        };

        let range = if index < HOTBAR_SIZE {
            HOTBAR_SIZE..INVENTORY_SIZE
        } else {
            0..HOTBAR_SIZE
        };

        if let Some(leftover) = move_stack_into_range(&mut self.slots, stack, range) {
            self.slots[index] = Some(leftover);
        }
    }

    pub fn left_click(&mut self, index: usize, cursor: &mut Option<ItemStack>) {
        if index >= INVENTORY_SIZE {
            return;
        }

        let held = cursor.take();
        let slot = self.slots[index].take();

        match (held, slot) {
            (None, None) => {}
            (None, Some(stack)) => {
                *cursor = Some(stack);
            }
            (Some(stack), None) => {
                self.slots[index] = Some(stack);
            }
            (Some(mut held), Some(mut slot)) => {
                if held.item == slot.item {
                    let capacity = slot.max_stack_size() - slot.count;
                    let moved = capacity.min(held.count);

                    slot.count += moved;
                    held.count -= moved;

                    self.slots[index] = Some(slot);

                    if held.count > 0 {
                        *cursor = Some(held);
                    }
                } else {
                    self.slots[index] = Some(held);
                    *cursor = Some(slot);
                }
            }
        }
    }

    pub fn right_click(&mut self, index: usize, cursor: &mut Option<ItemStack>) {
        if index >= INVENTORY_SIZE {
            return;
        }

        if cursor.is_none() {
            let Some(stack) = self.slots[index].take() else {
                return;
            };

            let taken = stack.count.div_ceil(2);
            let remaining = stack.count / 2;

            *cursor = Some(ItemStack::new(stack.item, taken));

            if remaining > 0 {
                self.slots[index] = Some(ItemStack::new(stack.item, remaining));
            }

            return;
        }

        let Some(mut held) = cursor.take() else {
            return;
        };

        match self.slots[index].take() {
            None => {
                self.slots[index] = Some(ItemStack::new(held.item, 1));
                held.count -= 1;

                if held.count > 0 {
                    *cursor = Some(held);
                }
            }
            Some(mut slot) if slot.item == held.item && slot.count < slot.max_stack_size() => {
                slot.count += 1;
                held.count -= 1;

                self.slots[index] = Some(slot);

                if held.count > 0 {
                    *cursor = Some(held);
                }
            }
            Some(slot) => {
                self.slots[index] = Some(slot);
                *cursor = Some(held);
            }
        }
    }

    pub fn collect_matching(&mut self, cursor: &mut Option<ItemStack>) {
        let Some(mut held) = cursor.take() else {
            return;
        };

        for slot in &mut self.slots {
            if held.count == held.max_stack_size() {
                break;
            }

            let Some(mut stack) = slot.take() else {
                continue;
            };

            if stack.item != held.item {
                *slot = Some(stack);
                continue;
            }

            let capacity = held.max_stack_size() - held.count;
            let moved = capacity.min(stack.count);

            held.count += moved;
            stack.count -= moved;

            if stack.count > 0 {
                *slot = Some(stack);
            }
        }

        *cursor = Some(held);
    }

    pub fn distribute_drag(
        &mut self,
        cursor: &mut Option<ItemStack>,
        visited: &[bool; INVENTORY_SIZE],
        mode: DragMode,
    ) {
        let Some(mut held) = cursor.take() else {
            return;
        };

        match mode {
            DragMode::OneEach => {
                for (index, visited) in visited.iter().copied().enumerate() {
                    if !visited || held.count == 0 {
                        continue;
                    }

                    match self.slots[index].take() {
                        None => {
                            self.slots[index] = Some(ItemStack::new(held.item, 1));
                            held.count -= 1;
                        }
                        Some(mut slot)
                            if slot.item == held.item && slot.count < slot.max_stack_size() =>
                        {
                            slot.count += 1;
                            held.count -= 1;
                            self.slots[index] = Some(slot);
                        }
                        Some(slot) => {
                            self.slots[index] = Some(slot);
                        }
                    }
                }
            }
            DragMode::Even => {
                let compatible = visited
                    .iter()
                    .copied()
                    .enumerate()
                    .filter(|(index, visited)| {
                        if !visited {
                            return false;
                        }

                        match self.slots[*index] {
                            None => true,
                            Some(slot) => {
                                slot.item == held.item && slot.count < slot.max_stack_size()
                            }
                        }
                    })
                    .count();

                if compatible == 0 {
                    *cursor = Some(held);
                    return;
                }

                let amount_per_slot = held.count / compatible as u8;

                if amount_per_slot == 0 {
                    *cursor = Some(held);
                    return;
                }

                for (index, visited) in visited.iter().copied().enumerate() {
                    if !visited || held.count == 0 {
                        continue;
                    }

                    match self.slots[index].take() {
                        None => {
                            let amount = amount_per_slot.min(held.count);

                            self.slots[index] = Some(ItemStack::new(held.item, amount));

                            held.count -= amount;
                        }
                        Some(mut slot) if slot.item == held.item => {
                            let capacity = slot.max_stack_size() - slot.count;

                            let amount = amount_per_slot.min(capacity).min(held.count);

                            slot.count += amount;
                            held.count -= amount;

                            self.slots[index] = Some(slot);
                        }
                        Some(slot) => {
                            self.slots[index] = Some(slot);
                        }
                    }
                }
            }
        }

        if held.count > 0 {
            *cursor = Some(held);
        }
    }

    pub fn consume_selected(&mut self, amount: u8) -> bool {
        if amount == 0 {
            return true;
        }

        let slot = &mut self.slots[self.selected_hotbar];

        let Some(mut stack) = slot.take() else {
            return false;
        };

        if stack.count < amount {
            *slot = Some(stack);
            return false;
        }

        stack.count -= amount;

        if stack.count > 0 {
            *slot = Some(stack);
        }

        true
    }
}

fn move_stack_into_range(
    slots: &mut [Option<ItemStack>; INVENTORY_SIZE],
    stack: ItemStack,
    range: std::ops::Range<usize>,
) -> Option<ItemStack> {
    let mut remaining = stack;

    for index in range.clone() {
        let Some(existing) = slots[index].as_mut() else {
            continue;
        };

        if existing.item != remaining.item {
            continue;
        }

        let capacity = existing.max_stack_size() - existing.count;
        let moved = capacity.min(remaining.count);

        existing.count += moved;
        remaining.count -= moved;

        if remaining.count == 0 {
            return None;
        }
    }

    for index in range {
        if slots[index].is_some() {
            continue;
        }

        slots[index] = Some(remaining);
        return None;
    }

    Some(remaining)
}
