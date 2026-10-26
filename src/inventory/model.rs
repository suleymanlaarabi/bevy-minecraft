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
    slots: super::slots::ItemSlots<SLOT_COUNT>,
}

impl Default for PlayerInventory {
    fn default() -> Self {
        let mut slots = [None; SLOT_COUNT];

        slots[0] = Some(ItemStack::new(ItemKind::Block(VoxelKind::Dirt), 2));

        Self {
            slots: slots.into(),
        }
    }
}

impl PlayerInventory {
    pub(crate) fn from_saved(slots: [Option<ItemStack>; SLOT_COUNT]) -> Self {
        Self {
            slots: slots.into(),
        }
    }
    /// Applies `edit` and flags the inventory as changed only when its contents differ,
    /// so no-op clicks and pickups do not refresh the slot views.
    pub fn edit<R>(inventory: &mut Mut<Self>, edit: impl FnOnce(&mut Self) -> R) -> R {
        let before = *inventory.slots;
        let result = edit(inventory.bypass_change_detection());
        if *inventory.slots != before {
            inventory.set_changed();
        }
        result
    }

    pub fn slot(&self, index: usize) -> Option<ItemStack> {
        self.slots.get(index).copied().flatten()
    }

    pub fn swap_slots(&mut self, a: usize, b: usize) {
        super::slots::swap(self, a, b);
    }

    pub fn insert(&mut self, stack: ItemStack) -> Option<ItemStack> {
        super::slots::insert(self, stack, 0..INVENTORY_SIZE)
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
        let range = if index >= INVENTORY_SIZE {
            0..INVENTORY_SIZE
        } else if index < HOTBAR_SIZE {
            HOTBAR_SIZE..INVENTORY_SIZE
        } else {
            0..HOTBAR_SIZE
        };
        super::slots::transfer(self, index, range);
    }

    fn is_armor_slot(index: usize) -> bool {
        (ARMOR_START..OFFHAND_SLOT).contains(&index)
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
        std::array::from_fn(|i| self.slots[CRAFT_START + i].take())
    }

    pub(crate) fn recipe(&self) -> Option<&'static super::crafting::Recipe> {
        super::crafting::find(&self.slots[CRAFT_START..]).map(super::crafting::get)
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

    pub fn consume(&mut self, index: usize, amount: u8) -> bool {
        let Some(stack) = self.slot(index) else {
            return amount == 0;
        };
        if stack.count() < amount {
            return false;
        }
        self.slots.set(
            index,
            (stack.count() > amount).then(|| ItemStack::new(stack.item(), stack.count() - amount)),
        );
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

impl super::slots::Slots for PlayerInventory {
    fn len(&self) -> usize {
        SLOT_COUNT
    }
    fn get(&self, index: usize) -> Option<ItemStack> {
        self.slots.get(index).copied().flatten()
    }
    fn set(&mut self, index: usize, stack: Option<ItemStack>) {
        if index < SLOT_COUNT {
            self.slots[index] = stack;
        }
    }
    fn capacity(&self, index: usize, item: ItemKind) -> u8 {
        if index >= SLOT_COUNT || (Self::is_armor_slot(index) && item.armor_slot() != Some(index)) {
            return 0;
        }
        Self::capacity(index, item)
    }
    fn collectible(&self, index: usize) -> bool {
        index < INVENTORY_SIZE
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HotbarSelection(pub usize);
impl HotbarSelection {
    pub fn stack(&self, inventory: &PlayerInventory) -> Option<ItemStack> {
        inventory.slot(self.0)
    }
}
#[derive(Component, Default, PartialEq, Eq)]
pub(crate) struct CraftingResult(pub Option<ItemStack>);

pub(crate) fn refresh_crafting(
    mut inventories: Query<(Ref<PlayerInventory>, &mut CraftingResult)>,
) {
    inventories
        .par_iter_mut()
        .for_each(|(inventory, mut result)| {
            if inventory.is_changed() {
                result.set_if_neq(CraftingResult(
                    inventory.recipe().map(|recipe| recipe.output),
                ));
            }
        });
}
