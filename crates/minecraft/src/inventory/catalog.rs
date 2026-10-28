//! Shared item metadata: registry components are compiled once into indexed tables.
use std::{fmt::Write, sync::OnceLock};

use bevy::prelude::BevyError;
use pumpkin_data::{
    attributes::Attributes, data_component_impl::*, effect::StatusEffect, item::Item,
    potion::Potion,
};

use super::{
    BLOCK_STACK_SIZE, ConsumableContents, ItemKind, ItemStack,
    model::{ITEM_ID_LIMIT, component},
};
use crate::survival::{ActiveEffect, ArmorStats};

pub(crate) enum ConsumeAction {
    Apply(Box<[ActiveEffect]>, f32),
    Remove(Box<[u8]>),
    Clear,
    Teleport(f32),
    Sound,
}

pub(crate) struct ConsumableProfile {
    pub duration: u16,
    pub drinking: bool,
    pub food: Option<&'static FoodImpl>,
    pub remainder: Option<ItemKind>,
    pub speed: f64,
    pub actions: Box<[ConsumeAction]>,
    contents: Box<[ActiveEffect]>,
    description: String,
}

impl ConsumableProfile {
    pub fn effects(&self, contents: ConsumableContents) -> impl Iterator<Item = ActiveEffect> + '_ {
        let (defaults, potion, stew) = match contents {
            ConsumableContents::Default => (&*self.contents, None, None),
            ConsumableContents::Potion(id) => (&[][..], Potion::from_id(id), None),
            ConsumableContents::Stew { effect, ticks } => (
                &[][..],
                None,
                Some(ActiveEffect {
                    id: effect,
                    amplifier: 0,
                    ticks,
                    show_icon: true,
                }),
            ),
        };
        defaults
            .iter()
            .copied()
            .chain(
                potion
                    .into_iter()
                    .flat_map(|potion| potion.effects)
                    .map(ActiveEffect::from),
            )
            .chain(stew)
    }
}

pub(crate) struct ItemProperties {
    pub block: Option<crate::voxel::VoxelKind>,
    pub max_stack_size: u8,
    pub max_damage: u32,
    pub attack_damage: f32,
    pub attack_wear: u32,
    /// Zero-based humanoid armor slot; inventory offsets are applied at the API boundary.
    pub armor_slot: Option<u8>,
    pub remainder: Option<ItemKind>,
    equip_sound: Option<crate::audio::SoundId>,
    consumable: Option<u16>,
}

struct Catalog {
    items: Box<[ItemProperties]>,
    armor: Box<[ArmorStats]>,
    consumables: Vec<ConsumableProfile>,
}

fn compiled() -> &'static Result<Catalog, String> {
    static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut armor = vec![ArmorStats::default(); ITEM_ID_LIMIT as usize].into_boxed_slice();
        let mut consumables = Vec::new();
        let items = (0..ITEM_ID_LIMIT)
            .map(|id| {
                let item = Item::from_id(id);
                let mut attack_damage = 1.0_f64;
                if let Some(modifiers) = item.and_then(component::<AttributeModifiersImpl>) {
                    let relevant = |modifier: &&Modifier| {
                        modifier.r#type.id == Attributes::ATTACK_DAMAGE.id
                            && matches!(
                                modifier.slot,
                                pumpkin_data::AttributeModifierSlot::Any
                                    | pumpkin_data::AttributeModifierSlot::Hand
                                    | pumpkin_data::AttributeModifierSlot::MainHand
                            )
                    };
                    for modifier in modifiers
                        .attribute_modifiers
                        .iter()
                        .filter(relevant)
                        .filter(|m| m.operation == Operation::AddValue)
                    {
                        attack_damage += modifier.amount;
                    }
                    let base = attack_damage;
                    for modifier in modifiers
                        .attribute_modifiers
                        .iter()
                        .filter(relevant)
                        .filter(|m| m.operation == Operation::AddMultipliedBase)
                    {
                        attack_damage += base * modifier.amount;
                    }
                    for modifier in modifiers
                        .attribute_modifiers
                        .iter()
                        .filter(relevant)
                        .filter(|m| m.operation == Operation::AddMultipliedTotal)
                    {
                        attack_damage *= 1.0 + modifier.amount;
                    }
                }
                let equipment = item.and_then(component::<EquippableImpl>);
                let armor_slot = equipment.and_then(|equipment| match equipment.slot {
                    EquipmentSlot::Head(_) => Some(0),
                    EquipmentSlot::Chest(_) => Some(1),
                    EquipmentSlot::Legs(_) => Some(2),
                    EquipmentSlot::Feet(_) => Some(3),
                    _ => None,
                });
                if armor_slot.is_some()
                    && let Some(modifiers) = item.and_then(component::<AttributeModifiersImpl>)
                {
                    for modifier in modifiers
                        .attribute_modifiers
                        .iter()
                        .filter(|modifier| modifier.operation == Operation::AddValue)
                    {
                        let stats = &mut armor[id as usize];
                        let amount = modifier.amount as f32;
                        match modifier.r#type.id {
                            id if id == Attributes::ARMOR.id => stats.protection += amount,
                            id if id == Attributes::ARMOR_TOUGHNESS.id => stats.toughness += amount,
                            id if id == Attributes::KNOCKBACK_RESISTANCE.id => {
                                stats.knockback_resistance += amount
                            }
                            _ => {}
                        }
                    }
                }
                let consumable = if let Some(source) = item.and_then(component::<ConsumableImpl>) {
                    let index = consumables.len() as u16;
                    consumables.push(compile_consumable(ItemKind(id), source)?);
                    Some(index)
                } else {
                    None
                };
                let remainder = if [
                    Item::MILK_BUCKET.id,
                    Item::WATER_BUCKET.id,
                    Item::LAVA_BUCKET.id,
                ]
                .contains(&id)
                {
                    Some(ItemKind(Item::BUCKET.id))
                } else if [Item::HONEY_BOTTLE.id, Item::DRAGON_BREATH.id].contains(&id) {
                    Some(ItemKind(Item::GLASS_BOTTLE.id))
                } else {
                    None
                };
                Ok(ItemProperties {
                    attack_damage: attack_damage.max(0.0) as f32,
                    attack_wear: item
                        .and_then(component::<WeaponImpl>)
                        .map_or(0, |weapon| weapon.item_damage_per_attack),
                    block: item
                        .and_then(|item| crate::voxel::VoxelKind::from_key(item.registry_key))
                        .filter(|kind| kind.is_itemizable()),
                    equip_sound: equipment.and_then(|equipment| match &equipment.equip_sound {
                        IdOr::Id(sound) => Some(*sound),
                        IdOr::Value(sound) => crate::audio::SoundId::from_name(
                            sound
                                .sound_name
                                .strip_prefix("minecraft:")
                                .unwrap_or(&sound.sound_name),
                        ),
                    }),
                    max_stack_size: item
                        .and_then(component::<MaxStackSizeImpl>)
                        .map_or(BLOCK_STACK_SIZE, |value| value.size),
                    max_damage: item
                        .and_then(component::<MaxDamageImpl>)
                        .map_or(0, |value| value.max_damage as u32),
                    armor_slot,
                    remainder,
                    consumable,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Catalog {
            items,
            armor,
            consumables,
        })
    })
}

fn catalog() -> &'static Catalog {
    compiled()
        .as_ref()
        .expect("Item metadata must compile before gameplay")
}

pub(crate) fn properties(item: ItemKind) -> &'static ItemProperties {
    &catalog().items[item.id() as usize]
}

pub(crate) fn consumable(item: ItemKind) -> Option<&'static ConsumableProfile> {
    properties(item)
        .consumable
        .map(|index| &catalog().consumables[index as usize])
}

pub(crate) fn equip_sound(item: ItemKind) -> Option<crate::audio::SoundId> {
    properties(item).equip_sound
}

pub(crate) fn armor(item: ItemKind) -> ArmorStats {
    catalog().armor[item.id() as usize]
}

pub(crate) fn initialize() -> Result<(), BevyError> {
    compiled()
        .as_ref()
        .map(|_| ())
        .map_err(|error| error.clone().into())
}

pub(crate) fn effect_name(id: u8) -> &'static str {
    static NAMES: OnceLock<[String; 256]> = OnceLock::new();
    &NAMES.get_or_init(|| {
        std::array::from_fn(|id| {
            <StatusEffect as IDSetContent>::from_id(id as u16)
                .map_or_else(String::new, |effect| readable(effect.minecraft_name))
        })
    })[id as usize]
}

fn readable(name: &str) -> String {
    name.trim_start_matches("minecraft:").replace('_', " ")
}

fn effect(id: &str, amplifier: i32, ticks: i32, show_icon: bool) -> Result<ActiveEffect, String> {
    let kind = StatusEffect::from_minecraft_name(id)
        .filter(|effect| crate::survival::effects::supported(effect.id))
        .ok_or_else(|| format!("Unsupported consumable effect: {id}"))?;
    Ok(ActiveEffect {
        id: kind.id,
        amplifier: amplifier.clamp(0, 4) as u8,
        ticks: ticks.clamp(1, 1_728_000) as u32,
        show_icon,
    })
}

fn describe_effect(text: &mut String, effect: &ActiveEffect) {
    write!(
        text,
        "\n{} {} · {}s",
        effect_name(effect.id),
        effect.amplifier + 1,
        effect.ticks / 20
    )
    .unwrap();
}

fn compile_consumable(
    item: ItemKind,
    source: &'static ConsumableImpl,
) -> Result<ConsumableProfile, String> {
    let food = item.component::<FoodImpl>();
    let mut description = String::new();
    if let Some(food) = food {
        write!(
            description,
            "\nNutrition +{} · Saturation +{:.1}",
            food.nutrition, food.saturation
        )
        .unwrap();
    }
    write!(description, "\nUse time: {:.1}s", source.consume_seconds).unwrap();
    let actions = source
        .effects
        .iter()
        .map(|action| {
            Ok(match action {
                ConsumeEffect::ApplyEffects((instances, chance)) => {
                    let effects = instances
                        .iter()
                        .map(|instance| {
                            effect(
                                &instance.effect_id,
                                instance.amplifier,
                                instance.duration,
                                instance.show_icon,
                            )
                        })
                        .collect::<Result<Box<[_]>, _>>()?;
                    for effect in &effects {
                        describe_effect(&mut description, effect);
                        write!(description, " · {:.0}%", chance * 100.0).unwrap();
                    }
                    ConsumeAction::Apply(effects, *chance)
                }
                ConsumeEffect::RemoveEffects(IDSet::IDs(ids)) => {
                    for effect in ids.iter() {
                        write!(description, "\nRemoves {}", effect_name(effect.id)).unwrap();
                    }
                    ConsumeAction::Remove(ids.iter().map(|effect| effect.id).collect())
                }
                ConsumeEffect::RemoveEffects(IDSet::Tag(_)) => {
                    return Err("Unsupported consumable effect tag in the item registry.".into());
                }
                ConsumeEffect::ClearAllEffects => {
                    description.push_str("\nClears all status effects");
                    ConsumeAction::Clear
                }
                ConsumeEffect::TeleportRandomly(diameter) => {
                    description.push_str("\nRandom safe teleport");
                    ConsumeAction::Teleport(*diameter)
                }
                ConsumeEffect::PlaySound(_) => ConsumeAction::Sound,
            })
        })
        .collect::<Result<_, String>>()?;
    let mut contents = Vec::new();
    if let Some(potion) = item
        .component::<PotionContentsImpl>()
        .and_then(|contents| contents.potion_id)
        .and_then(|id| u8::try_from(id).ok())
        .and_then(Potion::from_id)
    {
        contents.extend(potion.effects.iter().map(ActiveEffect::from));
    }
    if let Some(stew) = item.component::<SuspiciousStewEffectsImpl>() {
        for instance in stew.effects.iter() {
            contents.push(effect(&instance.effect, 0, instance.duration, true)?);
        }
    }
    Ok(ConsumableProfile {
        duration: source.consume_ticks().clamp(1, 1200) as u16,
        drinking: source.animation == ConsumeAnimation::Drink,
        food,
        remainder: item
            .component::<UseRemainderImpl>()
            .and_then(|data| data.remainder.as_deref())
            .and_then(|name| ItemKind::from_name(name.trim_start_matches("minecraft:"))),
        speed: item
            .component::<UseEffectsImpl>()
            .map_or(0.2, |data| data.speed_multiplier as f64),
        actions,
        contents: contents.into_boxed_slice(),
        description,
    })
}

pub(crate) fn description(stack: ItemStack) -> String {
    let mut text = readable(stack.item().name());
    if let Some(profile) = consumable(stack.item()) {
        text.push_str(&profile.description);
        for effect in profile.effects(stack.contents()) {
            describe_effect(&mut text, &effect);
        }
    }
    text
}
