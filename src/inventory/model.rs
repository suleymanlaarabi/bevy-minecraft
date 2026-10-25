use bevy::prelude::*;

use crate::voxel::VoxelKind;

pub const HOTBAR_SIZE: usize = 9;
pub const MAIN_INVENTORY_SIZE: usize = 27;
pub const INVENTORY_SIZE: usize = HOTBAR_SIZE + MAIN_INVENTORY_SIZE;
pub const BLOCK_STACK_SIZE: u8 = 64;
pub const ARMOR_START: usize = INVENTORY_SIZE;
pub const OFFHAND_SLOT: usize = ARMOR_START + 4;
pub const CRAFT_START: usize = OFFHAND_SLOT + 1;
pub const SLOT_COUNT: usize = CRAFT_START + 4;
pub const CRAFT_RESULT: usize = SLOT_COUNT;
/// Exclusive upper bound of item registry ids.
pub(crate) const ITEM_ID_LIMIT: u16 = 4096;

fn component<T: 'static>(item: &'static pumpkin_data::item::Item) -> Option<&'static T> {
    item.components
        .iter()
        .find_map(|(_, component)| component.as_any().downcast_ref::<T>())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Block(VoxelKind),
    Item(u16),
}

impl ItemKind {
    pub fn registry(self) -> &'static pumpkin_data::item::Item {
        match self {
            Self::Block(kind) => pumpkin_data::item::Item::from_id(kind.block().item_id).unwrap(),
            Self::Item(id) => pumpkin_data::item::Item::from_id(id).unwrap(),
        }
    }

    pub fn name(self) -> &'static str {
        self.registry().registry_key
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let item = pumpkin_data::item::Item::from_registry_key(name)?;
        if item.registry_key == "air" {
            return None;
        }
        Some(
            VoxelKind::from_key(item.registry_key)
                .filter(|kind| kind.is_itemizable())
                .map_or(Self::Item(item.id), Self::Block),
        )
    }

    /// First data component of type `T` on the item, as declared in the registry.
    pub fn component<T: 'static>(self) -> Option<&'static T> {
        component(self.registry())
    }

    /// Cached per registry id: the component scan is too slow for insert/merge loops.
    pub fn max_stack_size(self) -> u8 {
        use pumpkin_data::data_component_impl::MaxStackSizeImpl;
        static SIZES: std::sync::OnceLock<Box<[u8]>> = std::sync::OnceLock::new();
        let sizes = SIZES.get_or_init(|| {
            (0..ITEM_ID_LIMIT)
                .map(|id| {
                    pumpkin_data::item::Item::from_id(id)
                        .and_then(component::<MaxStackSizeImpl>)
                        .map_or(BLOCK_STACK_SIZE, |size| size.size)
                })
                .collect()
        });
        sizes[self.registry().id as usize]
    }

    pub fn armor_slot(self) -> Option<usize> {
        use pumpkin_data::data_component_impl::{EquipmentSlot, EquippableImpl};
        let equipment = self.component::<EquippableImpl>()?;
        Some(
            ARMOR_START
                + match equipment.slot {
                    EquipmentSlot::Head(_) => 0,
                    EquipmentSlot::Chest(_) => 1,
                    EquipmentSlot::Legs(_) => 2,
                    EquipmentSlot::Feet(_) => 3,
                    _ => return None,
                },
        )
    }

    pub fn is_valid(self) -> bool {
        match self {
            Self::Block(kind) => kind.is_itemizable(),
            Self::Item(id) => {
                pumpkin_data::item::Item::from_id(id).is_some_and(|item| item.registry_key != "air")
            }
        }
    }

    pub fn from_voxel(kind: VoxelKind) -> Option<Self> {
        crate::voxel::rules::item_form(kind).map(Self::Block)
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

    pub fn max_stack_size(self) -> u8 {
        self.item.max_stack_size()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragMode {
    Even,
    OneEach,
}

#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct PlayerInventory {
    slots: [Option<ItemStack>; SLOT_COUNT],
    selected_hotbar: usize,
    /// Recipe matched by the crafting grid, refreshed whenever the grid changes.
    recipe: Option<usize>,
}

impl Default for PlayerInventory {
    fn default() -> Self {
        let mut slots = [None; SLOT_COUNT];

        slots[0] = Some(ItemStack::new(ItemKind::Block(VoxelKind::Dirt), 2));

        Self {
            slots,
            selected_hotbar: 0,
            recipe: None,
        }
    }
}

impl PlayerInventory {
    pub(crate) fn from_saved(
        slots: [Option<ItemStack>; SLOT_COUNT],
        selected_hotbar: usize,
    ) -> Self {
        let mut inventory = Self {
            slots,
            selected_hotbar,
            recipe: None,
        };
        inventory.refresh_recipe();
        inventory
    }
    /// Applies `edit` and flags the inventory as changed only when its contents differ,
    /// so no-op clicks and pickups do not refresh the slot views.
    pub fn edit<R>(inventory: &mut Mut<Self>, edit: impl FnOnce(&mut Self) -> R) -> R {
        let before = Self::clone(inventory);
        let result = edit(inventory.bypass_change_detection());
        if inventory.slots[CRAFT_START..SLOT_COUNT] != before.slots[CRAFT_START..SLOT_COUNT] {
            inventory.bypass_change_detection().refresh_recipe();
        }
        if **inventory != before {
            inventory.set_changed();
        }
        result
    }

    pub fn slot(&self, index: usize) -> Option<ItemStack> {
        if index == CRAFT_RESULT {
            return self.recipe().map(|recipe| recipe.output);
        }
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
        if a >= SLOT_COUNT
            || b >= SLOT_COUNT
            || a == b
            || self.slots[a].is_some_and(|stack| !Self::fits(b, stack))
            || self.slots[b].is_some_and(|stack| !Self::fits(a, stack))
        {
            return;
        }

        self.slots.swap(a, b);
    }

    pub fn insert(&mut self, stack: ItemStack) -> Option<ItemStack> {
        move_stack_into_range(&mut self.slots, stack, 0..INVENTORY_SIZE)
    }

    pub fn quick_move(&mut self, index: usize) {
        if index >= SLOT_COUNT {
            return;
        }

        if index < INVENTORY_SIZE {
            if let Some(stack) = self.slots[index]
                && let Some(target) = stack.item().armor_slot()
                && self.slots[target].is_none()
            {
                self.slots[index] =
                    (stack.count > 1).then(|| ItemStack::new(stack.item, stack.count - 1));
                self.slots[target] = Some(ItemStack::new(stack.item, 1));
                return;
            }
        }
        let Some(stack) = self.slots[index].take() else {
            return;
        };

        let range = if index >= INVENTORY_SIZE {
            0..INVENTORY_SIZE
        } else if index < HOTBAR_SIZE {
            HOTBAR_SIZE..INVENTORY_SIZE
        } else {
            0..HOTBAR_SIZE
        };

        if let Some(leftover) = move_stack_into_range(&mut self.slots, stack, range) {
            self.slots[index] = Some(leftover);
        }
    }

    pub fn left_click(&mut self, index: usize, cursor: &mut Option<ItemStack>) {
        if index >= SLOT_COUNT {
            return;
        }

        if cursor.is_some_and(|stack| !Self::accepts(index, stack)) {
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
                let amount = stack.count.min(Self::capacity(index, stack.item));
                self.slots[index] = Some(ItemStack::new(stack.item, amount));
                *cursor = (stack.count > amount)
                    .then(|| ItemStack::new(stack.item, stack.count - amount));
            }
            (Some(mut held), Some(mut slot)) => {
                if held.item == slot.item {
                    let capacity = Self::capacity(index, slot.item) - slot.count;
                    let moved = capacity.min(held.count);

                    slot.count += moved;
                    held.count -= moved;

                    self.slots[index] = Some(slot);

                    if held.count > 0 {
                        *cursor = Some(held);
                    }
                } else if held.count <= Self::capacity(index, held.item) {
                    self.slots[index] = Some(held);
                    *cursor = Some(slot);
                } else {
                    self.slots[index] = Some(slot);
                    *cursor = Some(held);
                }
            }
        }
    }

    pub fn right_click(&mut self, index: usize, cursor: &mut Option<ItemStack>) {
        if index >= SLOT_COUNT {
            return;
        }

        if cursor.is_some_and(|stack| !Self::accepts(index, stack)) {
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
            Some(mut slot)
                if slot.item == held.item && slot.count < Self::capacity(index, slot.item) =>
            {
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

        // Only storage slots: the crafting grid, armor and offhand stay untouched.
        for slot in &mut self.slots[..INVENTORY_SIZE] {
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
        visited: &[bool; SLOT_COUNT],
        mode: DragMode,
    ) {
        let Some(mut held) = cursor.take() else {
            return;
        };

        match mode {
            DragMode::OneEach => {
                for (index, visited) in visited.iter().copied().enumerate() {
                    if !visited || held.count == 0 || !Self::accepts(index, held) {
                        continue;
                    }

                    match self.slots[index].take() {
                        None => {
                            self.slots[index] = Some(ItemStack::new(held.item, 1));
                            held.count -= 1;
                        }
                        Some(mut slot)
                            if slot.item == held.item
                                && slot.count < Self::capacity(index, slot.item) =>
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
                        if !visited || !Self::accepts(*index, held) {
                            return false;
                        }

                        match self.slots[*index] {
                            None => true,
                            Some(slot) => {
                                slot.item == held.item
                                    && slot.count < Self::capacity(*index, slot.item)
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
                    if !visited || held.count == 0 || !Self::accepts(index, held) {
                        continue;
                    }

                    match self.slots[index].take() {
                        None => {
                            let amount = amount_per_slot
                                .min(held.count)
                                .min(Self::capacity(index, held.item));

                            self.slots[index] = Some(ItemStack::new(held.item, amount));

                            held.count -= amount;
                        }
                        Some(mut slot) if slot.item == held.item => {
                            let capacity = Self::capacity(index, slot.item) - slot.count;

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

    const fn is_armor_slot(index: usize) -> bool {
        index >= ARMOR_START && index < OFFHAND_SLOT
    }

    pub fn capacity(index: usize, item: ItemKind) -> u8 {
        if Self::is_armor_slot(index) {
            1
        } else {
            item.max_stack_size()
        }
    }

    /// Whether `stack` may sit in slot `index` as a whole.
    pub fn fits(index: usize, stack: ItemStack) -> bool {
        Self::accepts(index, stack) && stack.count <= Self::capacity(index, stack.item)
    }

    pub(crate) fn craft_to_hotbar(&mut self, index: usize) {
        self.craft_once(|next, output| merge_output(&mut next.slots[index], output));
    }

    pub fn accepts(index: usize, stack: ItemStack) -> bool {
        if Self::is_armor_slot(index) {
            stack.item().armor_slot() == Some(index)
        } else {
            index < SLOT_COUNT
        }
    }

    pub(crate) fn take_crafting_inputs(&mut self) -> [Option<ItemStack>; 4] {
        let inputs = std::array::from_fn(|i| self.slots[CRAFT_START + i].take());
        self.recipe = None;
        inputs
    }

    pub(crate) fn recipe(&self) -> Option<&'static super::crafting::Recipe> {
        self.recipe.map(super::crafting::get)
    }

    fn refresh_recipe(&mut self) {
        self.recipe = super::crafting::find(&self.slots[CRAFT_START..SLOT_COUNT]);
    }

    /// Crafts once: `place` stores the output in a copy, committed only if the inputs and
    /// their remainders were consumed too.
    fn craft_once(&mut self, place: impl FnOnce(&mut Self, ItemStack) -> bool) -> bool {
        let Some(output) = self.recipe().map(|recipe| recipe.output) else {
            return false;
        };
        let mut next = self.clone();
        if !place(&mut next, output) || !next.consume_recipe() {
            return false;
        }
        *self = next;
        true
    }

    fn consume_recipe(&mut self) -> bool {
        for index in CRAFT_START..SLOT_COUNT {
            let Some(stack) = self.slots[index] else {
                continue;
            };
            self.slots[index] =
                (stack.count > 1).then(|| ItemStack::new(stack.item, stack.count - 1));
            let remainder = match stack.item.name() {
                "milk_bucket" | "water_bucket" | "lava_bucket" => Some("bucket"),
                "honey_bottle" | "dragon_breath" => Some("glass_bottle"),
                _ => None,
            };
            if let Some(item) = remainder.and_then(ItemKind::from_name) {
                let remainder = ItemStack::new(item, 1);
                if self.slots[index].is_none() {
                    self.slots[index] = Some(remainder);
                } else if self.insert(remainder).is_some() {
                    return false;
                }
            }
        }
        self.refresh_recipe();
        true
    }

    pub(crate) fn craft(&mut self, cursor: &mut Option<ItemStack>, quick: bool) {
        if quick {
            while self.craft_once(|next, output| next.insert(output).is_none()) {}
        } else {
            let mut held = *cursor;
            if self.craft_once(|_, output| merge_output(&mut held, output)) {
                *cursor = held;
            }
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

/// Adds `output` onto `target` when it is empty or holds a compatible, non-full stack.
fn merge_output(target: &mut Option<ItemStack>, output: ItemStack) -> bool {
    let count = target.map_or(0, |stack| stack.count) + output.count;
    if target.is_some_and(|stack| stack.item != output.item) || count > output.max_stack_size() {
        return false;
    }
    *target = Some(ItemStack::new(output.item, count));
    true
}

fn move_stack_into_range(
    slots: &mut [Option<ItemStack>; SLOT_COUNT],
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
