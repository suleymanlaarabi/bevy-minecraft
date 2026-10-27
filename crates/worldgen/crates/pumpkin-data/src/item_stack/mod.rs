use crate::data_component::DataComponent;
use crate::data_component::DataComponent::Enchantments;
use crate::data_component_impl::{
    BlocksAttacksImpl, ConsumableImpl, CustomDataImpl, DamageImpl, DataComponentImpl,
    EnchantmentsImpl, IDSet, MaxDamageImpl, MaxStackSizeImpl, Rarity, RarityImpl,
    SwingAnimationImpl, ToolImpl, UnbreakableImpl, UseCooldownImpl, get, get_mut, read_data,
};

use crate::item::Item;
use crate::recipes::RecipeResultStruct;
use crate::tag::Taggable;
use crate::{Block, Enchantment};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::GameMode;
use rand;
use std::borrow::Cow;
use std::cmp::{max, min};
use std::num::NonZero;
use std::sync::atomic::{AtomicU32, Ordering};

mod categories;

/// The outcome of a [`ItemStack::damage_item`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageResult {
    /// No damage was applied (zero/negative amount, not damageable, unbreakable,
    /// or Unbreaking negated every point).
    Untouched,
    /// Damage was applied and the item is still alive.
    Damaged,
    /// The item broke: one item was consumed from the stack (durability reset to 0),
    /// or the stack is now empty if it had only one item. Callers should always
    /// broadcast the break status — the client handles both cases correctly.
    Broken,
}

#[derive(Clone)]
pub struct ItemStack {
    pub item_count: u8,
    pub item: &'static Item,
    pub patch: Vec<(DataComponent, Option<Box<dyn DataComponentImpl>>)>,

    // unique ID for Bedrock network; don't serialize
    // Should always be a positive value for non-empty stacks
    pub uid: NonZero<i32>,
}

// impl Hash for ItemStack {
//     fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
//         self.item_count.hash(state);
//         self.item.id.hash(state);
//         self.patch.hash(state);
//     }
// }

/*
impl PartialEq for ItemStack {
    fn eq(&self, other: &Self) -> bool {
        self.item.id == other.item.id
    }
} */

pub struct ItemStackIdGenerator {
    counter: AtomicU32,
}

impl ItemStackIdGenerator {
    pub const fn new() -> Self {
        Self {
            counter: AtomicU32::new(1),
        }
    }

    pub fn next_id(&self) -> NonZero<i32> {
        // Wraps on overflow, which is what we want.
        let value = self.counter.fetch_add(1, Ordering::Relaxed);

        // Negative values are invalid; cycle through the positives
        let masked = value & 0x7FFFFFFF;

        if let Some(id) = NonZero::new(masked as i32) {
            id
        } else {
            // If we fetched 0 or 0x80000000, that's masked out as 0
            // Zero is invalid, so we just ask for the next ID to keep things simple/correct.
            self.next_id()
        }
    }
}

impl Default for ItemStackIdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

static ITEM_STACK_ID_GEN: ItemStackIdGenerator = ItemStackIdGenerator::new();

impl ItemStack {
    #[must_use]
    pub fn new(item_count: u8, item: &'static Item) -> Self {
        Self {
            item_count,
            item,
            patch: Vec::new(),

            uid: ITEM_STACK_ID_GEN.next_id(),
        }
    }

    #[must_use]
    pub fn new_with_component(
        item_count: u8,
        item: &'static Item,
        component: Vec<(DataComponent, Option<Box<dyn DataComponentImpl>>)>,
    ) -> Self {
        Self {
            item_count,
            item,
            patch: component,

            uid: ITEM_STACK_ID_GEN.next_id(),
        }
    }

    /// Ignore the Bedrock UID for this `ItemStack`.
    /// This constructor is intended for Java-only item stacks, where the network UID
    /// is not required.
    #[must_use]
    pub const fn static_new_java(item_count: u8, item: &'static Item) -> Self {
        Self {
            item_count,
            item,
            patch: Vec::new(),

            uid: match NonZero::new(1) {
                Some(v) => v,
                None => panic!("1 is non-zero"),
            },
        }
    }

    #[must_use]
    pub fn get_data_component<T: DataComponentImpl + 'static>(&self) -> Option<&T> {
        let to_get_id = &T::get_enum();
        for (id, component) in &self.patch {
            if id == to_get_id {
                return component
                    .as_ref()
                    .map(|component| get::<T>(component.as_ref()));
            }
        }
        for (id, component) in self.item.components {
            if id == to_get_id {
                return Some(get::<T>(*component));
            }
        }
        None
    }
    #[must_use]
    pub fn get_data_component_mut<T: DataComponentImpl + 'static>(&mut self) -> Option<&mut T> {
        let to_get_id = T::get_enum();
        if let Some(index) = self.patch.iter().position(|(id, _)| *id == to_get_id) {
            return self.patch[index]
                .1
                .as_mut()
                .map(|component| get_mut::<T>(component.as_mut()));
        }

        // If not in patch, clone from item to patch and return mut
        let mut cloned = None;
        for (id, component) in self.item.components {
            if *id == to_get_id {
                cloned = Some((*id, Some(component.clone_dyn())));
                break;
            }
        }
        if let Some((id, component)) = cloned {
            self.patch.push((id, component));
            return self
                .patch
                .last_mut()?
                .1
                .as_mut()
                .map(|c| get_mut::<T>(c.as_mut()));
        }
        None
    }

    #[must_use]
    pub fn has_data_component(&self, to_get_id: DataComponent) -> bool {
        for (id, component) in &self.patch {
            if *id == to_get_id {
                return component.is_some();
            }
        }
        for (id, _) in self.item.components {
            if *id == to_get_id {
                return true;
            }
        }
        false
    }

    pub fn has_enchantments(&self) -> bool {
        self.get_data_component::<EnchantmentsImpl>()
            .is_some_and(|e| !e.enchantment.is_empty())
    }

    pub fn add_enchantment(&mut self, enchantment: &'static Enchantment, level: u16) {
        if let Some(enchantments) = self.get_data_component_mut::<EnchantmentsImpl>() {
            let mut new_vec = enchantments.enchantment.to_vec();
            new_vec.push((enchantment, level as i32));
            enchantments.enchantment = Cow::Owned(new_vec);
        } else {
            let enchantments = EnchantmentsImpl {
                enchantment: Cow::Owned(vec![(enchantment, level as i32)]),
            };
            self.patch
                .push((DataComponent::Enchantments, Some(Box::new(enchantments))));
        }
    }

    pub fn set_lore(&mut self, lines: Vec<pumpkin_util::text::TextComponent>) {
        let lore = Some(Box::new(crate::data_component_impl::LoreImpl { lines }) as _);
        if let Some((_, component)) = self
            .patch
            .iter_mut()
            .find(|(id, _)| *id == DataComponent::Lore)
        {
            *component = lore;
        } else {
            self.patch.push((DataComponent::Lore, lore));
        }
    }

    pub fn set_data_component<T: DataComponentImpl + 'static>(&mut self, component: T) {
        let to_set_id = T::get_enum();
        let boxed = Some(Box::new(component) as _);
        if let Some((_, c)) = self.patch.iter_mut().find(|(id, _)| *id == to_set_id) {
            *c = boxed;
        } else {
            self.patch.push((to_set_id, boxed));
        }
    }

    pub fn remove_data_component(&mut self, to_remove_id: DataComponent) {
        if let Some((_, c)) = self.patch.iter_mut().find(|(id, _)| *id == to_remove_id) {
            *c = None;
        } else {
            self.patch.push((to_remove_id, None));
        }
    }

    pub fn add_lore(&mut self, line: pumpkin_util::text::TextComponent) {
        let mut lines = self
            .get_data_component::<crate::data_component_impl::LoreImpl>()
            .map_or_else(Vec::new, |lore| lore.lines.clone());
        lines.push(line);
        self.set_lore(lines);
    }

    pub const EMPTY: &'static Self = &Self {
        item_count: 0,
        item: &Item::AIR,
        patch: Vec::new(),

        uid: NonZero::<i32>::MIN, // white lie - Bedrock `uid` is never sent if the stack is empty
    };

    #[must_use]
    pub fn split_off(&mut self, amount: u8) -> Self {
        let count = amount.min(self.item_count);
        let result = self.copy_with_count(count);
        self.decrement(count);
        result
    }

    #[must_use]
    pub fn get_max_stack_size(&self) -> u8 {
        self.get_data_component::<MaxStackSizeImpl>()
            .map_or(1, |value| value.size)
    }

    #[must_use]
    pub fn get_max_damage(&self) -> Option<i32> {
        self.get_data_component::<MaxDamageImpl>()
            .map(|value| value.max_damage)
    }

    #[must_use]
    pub fn get_use_cooldown(&self) -> Option<&UseCooldownImpl> {
        self.get_data_component::<UseCooldownImpl>()
    }

    #[must_use]
    pub fn get_damage(&self) -> i32 {
        self.get_data_component::<DamageImpl>()
            .map_or(0, |value| value.damage)
    }

    #[must_use]
    pub fn get_enchantment_level(&self, enchantment: &'static Enchantment) -> i32 {
        let Some(data) = self.get_data_component::<EnchantmentsImpl>() else {
            return 0;
        };
        for (enc, level) in data.enchantment.iter() {
            if *enc == enchantment {
                return *level;
            }
        }
        0
    }

    #[must_use]
    pub fn is_unbreakable(&self) -> bool {
        self.get_data_component::<UnbreakableImpl>().is_some()
    }

    pub fn set_damage(&mut self, damage: i32) {
        let damage = damage.max(0);
        if damage == 0 {
            self.patch.retain(|(id, _)| *id != DataComponent::Damage);
            return;
        }

        for (id, component) in &mut self.patch {
            if *id == DataComponent::Damage {
                *component = Some(DamageImpl { damage }.to_dyn());
                return;
            }
        }

        self.patch
            .push((DataComponent::Damage, Some(DamageImpl { damage }.to_dyn())));
    }

    #[must_use]
    pub fn is_damageable(&self) -> bool {
        self.get_max_damage().unwrap_or(0) > 0
    }

    #[must_use]
    pub fn next_damage_will_break(&self) -> bool {
        self.is_damageable()
            && !self.is_unbreakable()
            && self.get_damage() >= self.get_max_damage().unwrap_or(0) - 1
    }

    pub fn repair_item(&mut self, amount: i32) -> i32 {
        if amount <= 0 {
            return 0;
        }
        let damage = self.get_damage();
        if damage <= 0 {
            return 0;
        }
        let repaired = amount.min(damage);
        self.set_damage(damage - repaired);
        repaired
    }

    /// Core logic: apply Unbreaking chance with precomputed armor category and level.
    /// Extracted for use in damage_item where these values are hoisted outside the loop.
    /// Private to prevent incorrect usage; only call through damage_item.
    fn should_apply_durability_damage_with(is_armor: bool, unbreaking_level: i32) -> bool {
        if unbreaking_level <= 0 {
            return true;
        }

        let mut damage = 1.0f32;
        Enchantment::UNBREAKING.modify_durability_damage(unbreaking_level, is_armor, &mut damage);
        damage > 0.0
    }

    /// Apply durability damage to this item and return the outcome.
    /// Callers must check the return value to handle break broadcasts and item stack updates.
    /// TODO: Restore `#[must_use]` once all callsites (esp. tool/mob block-hit/damage sites)
    /// implement proper `DamageResult::Broken` handling instead of suppressing with `let _ =`.
    /// Without this enforcement, the fix is incomplete vs vanilla break behavior.
    #[must_use]
    pub fn damage_item(&mut self, amount: i32) -> DamageResult {
        if amount <= 0 || !self.is_damageable() || self.is_unbreakable() {
            return DamageResult::Untouched;
        }

        let max_damage = self.get_max_damage().unwrap_or(0);
        if max_damage <= 0 {
            return DamageResult::Untouched;
        }

        // Hoist armor check and enchantment level outside loop to avoid repeated lookups.
        let is_armor = self.is_armor();
        let unbreaking_level = self.get_enchantment_level(&Enchantment::UNBREAKING);
        let mut applied = 0;
        // TODO: Short-circuit once applied >= (max_damage - current_damage) to avoid
        // iterating the full amount for high-damage hits on high-durability items.
        for _ in 0..amount {
            if Self::should_apply_durability_damage_with(is_armor, unbreaking_level) {
                applied += 1;
            }
        }

        if applied <= 0 {
            return DamageResult::Untouched;
        }

        let new_damage = self.get_damage().saturating_add(applied);
        if new_damage >= max_damage {
            // Vanilla behavior: breaking consumes one item from the stack and resets
            // durability to 0. A single damage call never breaks more than one item,
            // regardless of the damage amount. This matches vanilla item stack behavior.
            if self.item_count > 1 {
                self.item_count = self.item_count.saturating_sub(1);
                self.set_damage(0);
            } else {
                *self = Self::EMPTY.clone();
            }
            return DamageResult::Broken;
        }

        self.set_damage(new_damage);
        DamageResult::Damaged
    }

    #[must_use]
    pub fn get_max_use_time(&self) -> i32 {
        if let Some(value) = self.get_data_component::<ConsumableImpl>() {
            return value.consume_ticks();
        }
        if self.get_data_component::<BlocksAttacksImpl>().is_some() {
            return 72000;
        }
        0
    }

    #[must_use]
    pub const fn get_item(&self) -> &'static Item {
        if self.is_empty() {
            &Item::AIR
        } else {
            self.item
        }
    }

    #[must_use]
    pub fn is_stackable(&self) -> bool {
        self.get_max_stack_size() > 1 // TODO: && (!this.isDamageable() || !this.isDamaged());
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.item_count == 0 || self.item.id == Item::AIR.id
    }

    pub fn set_custom_name(&mut self, name: String) {
        use crate::data_component_impl::CustomNameImpl;
        let component = Some(
            CustomNameImpl {
                name: pumpkin_util::text::TextComponent::text(name),
            }
            .to_dyn(),
        );
        if let Some(pos) = self
            .patch
            .iter()
            .position(|(id, _)| *id == DataComponent::CustomName)
        {
            self.patch[pos].1 = component;
        } else {
            self.patch.push((DataComponent::CustomName, component));
        }
    }

    #[must_use]
    pub fn has_custom_name(&self) -> bool {
        self.get_data_component::<crate::data_component_impl::CustomNameImpl>()
            .is_some()
    }

    #[must_use]
    pub fn get_custom_name(&self) -> Option<&pumpkin_util::text::TextComponent> {
        self.get_data_component::<crate::data_component_impl::CustomNameImpl>()
            .map(|c| &c.name)
    }

    pub fn remove_custom_name(&mut self) {
        self.patch
            .retain(|(id, _)| *id != DataComponent::CustomName);
    }

    #[must_use]
    pub fn get_hover_name(&self) -> String {
        if let Some(custom_name) =
            self.get_data_component::<crate::data_component_impl::CustomNameImpl>()
        {
            return custom_name.name.clone().get_text();
        }
        if let Some(item_name) =
            self.get_data_component::<crate::data_component_impl::ItemNameImpl>()
        {
            return item_name.name.to_string();
        }
        self.item.registry_key.to_string()
    }

    #[must_use]
    pub fn get_repair_cost(&self) -> i32 {
        self.get_data_component::<crate::data_component_impl::RepairCostImpl>()
            .map_or(0, |value| value.cost)
    }

    pub fn set_repair_cost(&mut self, cost: i32) {
        if cost <= 0 {
            self.patch
                .retain(|(id, _)| *id != DataComponent::RepairCost);
            return;
        }
        self.set_data_component(crate::data_component_impl::RepairCostImpl { cost });
    }

    #[must_use]
    pub fn is_valid_repair_item(&self, repair_item: &ItemStack) -> bool {
        let repairable = self.get_data_component::<crate::data_component_impl::RepairableImpl>();
        repairable.is_some_and(|r| r.is_valid_repair_item(repair_item))
    }

    #[must_use]
    pub fn get_swing_animation(&self) -> SwingAnimationImpl {
        self.get_data_component::<SwingAnimationImpl>()
            .copied()
            .unwrap_or(SwingAnimationImpl::DEFAULT)
    }

    #[must_use]
    pub fn get_rarity(&self) -> Rarity {
        let base = self
            .get_data_component::<RarityImpl>()
            .map_or(Rarity::Common, |r| r.rarity);
        if !self.has_enchantments() {
            return base;
        }
        match base {
            Rarity::Common | Rarity::Uncommon => Rarity::Rare,
            Rarity::Rare => Rarity::Epic,
            Rarity::Epic => Rarity::Epic,
        }
    }

    #[must_use]
    pub fn custom_data_compound(&self) -> Option<&NbtCompound> {
        self.get_data_component::<CustomDataImpl>()
            .map(|custom_data| &custom_data.data)
    }

    pub fn set_custom_data(&mut self, namespace: &str, key: &str, value: NbtTag) {
        let mut custom_data = self
            .get_data_component::<CustomDataImpl>()
            .map_or_else(NbtCompound::new, |custom_data| custom_data.data.clone());

        let mut namespace_data = custom_data
            .child_tags
            .remove(namespace)
            .and_then(|tag| match tag {
                NbtTag::Compound(compound) => Some(compound),
                _ => None,
            })
            .unwrap_or_default();

        namespace_data.child_tags.insert(key.into(), value);
        custom_data
            .child_tags
            .insert(namespace.into(), NbtTag::Compound(namespace_data));

        self.set_custom_data_component(custom_data);
    }

    fn set_custom_data_component(&mut self, custom_data: NbtCompound) {
        let component = Some(CustomDataImpl { data: custom_data }.to_dyn());
        if let Some((_, data)) = self
            .patch
            .iter_mut()
            .find(|(id, _)| *id == DataComponent::CustomData)
        {
            *data = component;
        } else {
            self.patch.push((DataComponent::CustomData, component));
        }
    }

    pub fn get_custom_data(&self, namespace: &str, key: &str) -> Option<NbtTag> {
        self.custom_data_compound()?
            .get(namespace)?
            .extract_compound()?
            .get(key)
            .cloned()
    }

    pub fn remove_custom_data(&mut self, namespace: &str, key: &str) {
        let Some(mut custom_data) = self
            .get_data_component::<CustomDataImpl>()
            .map(|custom_data| custom_data.data.clone())
        else {
            return;
        };

        let Some(NbtTag::Compound(mut namespace_data)) = custom_data.child_tags.remove(namespace)
        else {
            return;
        };

        namespace_data.child_tags.remove(key);
        if !namespace_data.is_empty() {
            custom_data
                .child_tags
                .insert(namespace.into(), NbtTag::Compound(namespace_data));
        }

        if custom_data.is_empty() {
            self.patch
                .retain(|(id, _)| *id != DataComponent::CustomData);
        } else {
            self.set_custom_data_component(custom_data);
        }
    }

    #[must_use]
    pub fn has_custom_data(&self, namespace: &str, key: &str) -> bool {
        self.get_custom_data(namespace, key).is_some()
    }

    #[must_use]
    pub fn split(&mut self, amount: u8) -> Self {
        let min = amount.min(self.item_count);
        let stack = self.copy_with_count(min);
        self.decrement(min);
        stack
    }

    #[must_use]
    pub fn split_unless_creative(&mut self, gamemode: GameMode, amount: u8) -> Self {
        let min = amount.min(self.item_count);
        let stack = self.copy_with_count(min);
        if gamemode != GameMode::Creative {
            self.decrement(min);
        }
        stack
    }

    #[must_use]
    pub fn copy_with_count(&self, count: u8) -> Self {
        let mut stack = self.clone();
        stack.uid = ITEM_STACK_ID_GEN.next_id();
        stack.item_count = count;
        stack
    }

    pub const fn set_count(&mut self, count: u8) {
        self.item_count = count;
    }

    pub fn decrement_unless_creative(&mut self, gamemode: GameMode, amount: u8) {
        if gamemode != GameMode::Creative {
            self.item_count = self.item_count.saturating_sub(amount);
            if self.item_count == 0 {
                self.clear();
            }
        }
    }

    pub const fn decrement(&mut self, amount: u8) {
        self.item_count = self.item_count.saturating_sub(amount);
    }

    pub const fn increment(&mut self, amount: u8) {
        self.item_count = self.item_count.saturating_add(amount);
    }

    /// Completely resets the stack to air
    pub fn clear(&mut self) {
        *self = Self::EMPTY.clone();
    }

    pub fn enchant(&mut self, enchantment: &'static Enchantment, level: i32) {
        if level <= 0 {
            return;
        }
        let level = min(level, 255);
        if let Some(data) = self.get_data_component_mut::<EnchantmentsImpl>() {
            for (enc, old_level) in data.enchantment.to_mut() {
                if *enc == enchantment {
                    *old_level = max(*old_level, level);
                    return;
                }
            }
            data.enchantment.to_mut().push((enchantment, level));
        } else {
            self.patch.push((
                Enchantments,
                Some(
                    EnchantmentsImpl {
                        enchantment: Cow::Owned(vec![(enchantment, level)]),
                    }
                    .to_dyn(),
                ),
            ));
        }
    }

    #[must_use]
    pub fn are_items_and_components_equal(&self, other: &Self) -> bool {
        // Items must match
        if self.item != other.item {
            return false;
        }

        if self.patch.len() != other.patch.len() {
            return false;
        }

        for (id, data) in &self.patch {
            let mut not_found = true;
            'out: for (other_id, other_data) in &other.patch {
                if id == other_id {
                    if let (Some(data), Some(other_data)) = (data, other_data) {
                        if !data.equal(other_data.as_ref()) {
                            return false;
                        }
                        not_found = false;
                        break 'out;
                    } else if data.is_none() && other_data.is_none() {
                        not_found = false;
                        break 'out;
                    }
                    return false;
                }
            }
            if not_found {
                return false;
            }
        }

        true
    }

    #[must_use]
    pub fn are_equal(&self, other: &Self) -> bool {
        self.item_count == other.item_count && self.are_items_and_components_equal(other)
    }

    /// Determines the mining speed for a block based on tool rules.
    /// Direct matches return immediately, tagged blocks are checked separately.
    /// If no match is found, returns the tool's default mining speed or `1.0`.
    #[must_use]
    pub fn get_speed(&self, block: &'static Block) -> f32 {
        // No tool? Use default speed
        if let Some(tool) = self.get_data_component::<ToolImpl>() {
            for rule in tool.rules.iter() {
                // Skip if speed is not set
                let Some(speed) = rule.speed else {
                    continue;
                };
                match &rule.blocks {
                    IDSet::Tag(tag) => {
                        if block.is_tagged_with(tag).unwrap_or(false) {
                            return speed;
                        }
                    }
                    IDSet::IDs(blocks) => {
                        if blocks.contains(&block) {
                            return speed;
                        }
                    }
                }
            }
            tool.default_mining_speed
        } else {
            1.0
        }
    }

    /// Determines if a tool is valid for block drops based on tool rules.
    /// Direct matches return immediately, while tagged blocks are checked separately.
    #[must_use]
    pub fn is_correct_for_drops(&self, block: &'static Block) -> bool {
        if let Some(tool) = self.get_data_component::<ToolImpl>() {
            for rule in tool.rules.iter() {
                // Skip if speed is not set
                let Some(correct) = rule.correct_for_drops else {
                    continue;
                };
                match &rule.blocks {
                    IDSet::Tag(tag) => {
                        if block.is_tagged_with(tag).unwrap_or(false) {
                            return correct;
                        }
                    }
                    IDSet::IDs(blocks) => {
                        if blocks.contains(&block) {
                            return correct;
                        }
                    }
                }
            }
        }
        false
    }

    pub fn write_item_stack(&self, compound: &mut NbtCompound) {
        // Minecraft 1.21.4 uses "id" as string with namespaced ID (minecraft:diamond_sword)
        compound.put_string("id", format!("minecraft:{}", self.item.registry_key));
        compound.put_int("count", self.item_count as i32);

        // Create a tag compound for additional data
        let mut tag = NbtCompound::new();

        for (id, data) in &self.patch {
            if !id.is_persistent() {
                continue;
            }
            if let Some(data) = data {
                tag.put(id.to_name(), data.write_data());
            } else {
                let name = '!'.to_string() + id.to_name();
                tag.put(name.as_str(), NbtCompound::new());
            }
        }

        // Store custom data like enchantments, display name, etc. would go here
        compound.put_compound("components", tag);
    }

    #[must_use]
    pub fn read_item_stack(compound: &NbtCompound) -> Option<Self> {
        // Get ID, which is a string like "minecraft:diamond_sword"
        let full_id = compound.get_string("id")?;

        // Remove the "minecraft:" prefix if present
        let registry_key = full_id.strip_prefix("minecraft:").unwrap_or(full_id);

        // Try to get item by registry key
        let item = Item::from_registry_key(registry_key)?;

        let count = compound.get_int("count")? as u8;

        // Create the item stack
        let mut item_stack = Self::new(count, item);

        // Process any additional data in the components compound
        if let Some(tag) = compound.get_compound("components") {
            for (name, data) in &tag.child_tags {
                if let Some(name) = name.strip_prefix("!") {
                    let id = DataComponent::try_from_name(name)?;
                    if id.is_persistent() {
                        item_stack.patch.push((id, None));
                    }
                } else {
                    let id = DataComponent::try_from_name(name)?;
                    if id.is_persistent() {
                        item_stack.patch.push((id, Some(read_data(id, data)?)));
                    }
                }
            }
        }

        Some(item_stack)
    }
}

impl From<&RecipeResultStruct> for ItemStack {
    fn from(value: &RecipeResultStruct) -> Self {
        Self::new(
            value.count,
            Item::from_registry_key(value.id.strip_prefix("minecraft:").unwrap_or(value.id))
                .unwrap_or(&Item::AIR),
        )
    }
}
