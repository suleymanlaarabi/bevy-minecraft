use bevy::prelude::*;

use crate::voxel::VoxelKind;

pub const HOTBAR_SIZE: usize = 9;
pub const MAIN_INVENTORY_SIZE: usize = 27;
pub const INVENTORY_SIZE: usize = HOTBAR_SIZE + MAIN_INVENTORY_SIZE;
pub const BLOCK_STACK_SIZE: u8 = 64;
pub const ARMOR_START: usize = INVENTORY_SIZE;
pub const OFFHAND_SLOT: usize = ARMOR_START + 4;
pub const CRAFT_START: usize = OFFHAND_SLOT + 1;
pub const SLOT_COUNT: usize = CRAFT_START;
pub const CRAFT_RESULT: usize = CRAFT_START + 9;
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

    pub fn max_stack_size(self) -> u8 {
        properties()[self.registry().id as usize].max_stack_size
    }

    pub fn max_damage(self) -> u32 {
        properties()[self.registry().id as usize].max_damage
    }

    pub fn armor_slot(self) -> Option<usize> {
        properties()[self.registry().id as usize].armor_slot
    }

    pub(crate) fn crafting_remainder(self) -> Option<ItemKind> {
        properties()[self.registry().id as usize].remainder
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

struct ItemProperties {
    max_stack_size: u8,
    max_damage: u32,
    armor_slot: Option<usize>,
    remainder: Option<ItemKind>,
}

fn properties() -> &'static [ItemProperties] {
    use pumpkin_data::{data_component_impl::*, item::Item};
    static PROPERTIES: std::sync::OnceLock<Box<[ItemProperties]>> = std::sync::OnceLock::new();
    PROPERTIES.get_or_init(|| {
        (0..ITEM_ID_LIMIT)
            .map(|id| {
                let item = Item::from_id(id);
                let armor_slot = item
                    .and_then(component::<EquippableImpl>)
                    .and_then(|equipment| {
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
                    });
                let remainder = if [
                    Item::MILK_BUCKET.id,
                    Item::WATER_BUCKET.id,
                    Item::LAVA_BUCKET.id,
                ]
                .contains(&id)
                {
                    Some(ItemKind::Item(Item::BUCKET.id))
                } else if [Item::HONEY_BOTTLE.id, Item::DRAGON_BREATH.id].contains(&id) {
                    Some(ItemKind::Item(Item::GLASS_BOTTLE.id))
                } else {
                    None
                };
                ItemProperties {
                    max_stack_size: item
                        .and_then(component::<MaxStackSizeImpl>)
                        .map_or(BLOCK_STACK_SIZE, |value| value.size),
                    max_damage: item
                        .and_then(component::<MaxDamageImpl>)
                        .map_or(0, |value| value.max_damage as u32),
                    armor_slot,
                    remainder,
                }
            })
            .collect()
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemStack {
    item: ItemKind,
    count: u8,
    damage: u32,
}

impl ItemStack {
    pub fn new(item: ItemKind, count: u8) -> Self {
        assert!(item.is_valid());
        assert!(count > 0);
        assert!(count <= item.max_stack_size());

        Self {
            item,
            count,
            damage: 0,
        }
    }

    pub const fn item(self) -> ItemKind {
        self.item
    }

    pub const fn count(self) -> u8 {
        self.count
    }

    pub const fn damage(self) -> u32 {
        self.damage
    }

    pub fn with_damage(mut self, damage: u32) -> Option<Self> {
        let maximum = self.item.max_damage();
        if (maximum == 0 && damage != 0) || (maximum != 0 && damage >= maximum) {
            return None;
        }
        self.damage = damage;
        Some(self)
    }

    pub fn with_count(mut self, count: u8) -> Option<Self> {
        if count == 0 {
            return None;
        }
        assert!(count <= self.max_stack_size());
        self.count = count;
        Some(self)
    }

    pub fn can_merge(self, other: Self) -> bool {
        self.item == other.item && self.damage == other.damage
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
        self.slots.as_slice().get(index).copied().flatten()
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
                self.slots[index] = stack.with_count(stack.count - 1);
                self.slots[target] = stack.with_count(1);
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

    pub fn accepts(index: usize, stack: ItemStack) -> bool {
        if Self::is_armor_slot(index) {
            stack.item().armor_slot() == Some(index)
        } else {
            index < SLOT_COUNT
        }
    }

    pub(crate) fn snapshot(&self) -> [Option<ItemStack>; SLOT_COUNT] {
        *self.slots
    }

    pub(crate) fn replace_slots(&mut self, slots: [Option<ItemStack>; SLOT_COUNT]) {
        if *self.slots != slots {
            *self.slots = slots;
        }
    }

    pub(crate) fn damage_tool(&mut self, index: usize, amount: u32) {
        if let Some(stack) = self.slot(index)
            && stack.item().max_damage() > 0
        {
            self.slots.set(
                index,
                stack.with_damage(stack.damage().saturating_add(amount)),
            );
        }
    }

    pub fn consume(&mut self, index: usize, amount: u8) -> bool {
        let Some(stack) = self.slot(index) else {
            return amount == 0;
        };
        if stack.count() < amount {
            return false;
        }
        self.slots
            .set(index, stack.with_count(stack.count() - amount));
        true
    }
}

impl super::slots::Slots for PlayerInventory {
    fn len(&self) -> usize {
        SLOT_COUNT
    }
    fn get(&self, index: usize) -> Option<ItemStack> {
        self.slots.as_slice().get(index).copied().flatten()
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
