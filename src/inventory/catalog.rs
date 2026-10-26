//! Shared item metadata: registry components are compiled once into indexed tables.
use std::{fmt::Write, sync::OnceLock};

use bevy::prelude::BevyError;
use pumpkin_data::{
    attributes::Attributes, data_component_impl::*, effect::StatusEffect, item::Item,
    potion::Potion,
};

use super::{
    ARMOR_START, BLOCK_STACK_SIZE, ConsumableContents, ItemKind, ItemStack,
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

pub(super) struct ItemProperties {
    pub max_stack_size: u8,
    pub max_damage: u32,
    pub armor_slot: Option<usize>,
    pub remainder: Option<ItemKind>,
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
                    consumables.push(compile_consumable(ItemKind::Item(id), source)?);
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
                    Some(ItemKind::Item(Item::BUCKET.id))
                } else if [Item::HONEY_BOTTLE.id, Item::DRAGON_BREATH.id].contains(&id) {
                    Some(ItemKind::Item(Item::GLASS_BOTTLE.id))
                } else {
                    None
                };
                Ok(ItemProperties {
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

pub(super) fn properties(item: ItemKind) -> &'static ItemProperties {
    &catalog().items[item.registry().id as usize]
}

pub(crate) fn consumable(item: ItemKind) -> Option<&'static ConsumableProfile> {
    properties(item)
        .consumable
        .map(|index| &catalog().consumables[index as usize])
}

pub(crate) fn armor(item: ItemKind) -> ArmorStats {
    catalog().armor[item.registry().id as usize]
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
