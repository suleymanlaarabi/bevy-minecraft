use bevy::prelude::*;
use pumpkin_data::{
    damage::DamageType,
    effect::StatusEffect as E,
    tag::{self, Taggable},
};

use super::{ActiveEffects, ArmorStats, DamageState, Health, Nutrition, UsingItem};
use crate::{
    character::{CharacterMotion, CharacterMovement},
    inventory::{ARMOR_START, PlayerInventory},
};

/// All damage enters through this buffered request, including environmental damage.
#[derive(Message, Clone, Debug)]
pub struct DamageRequest {
    pub target: Entity,
    pub source: &'static DamageType,
    pub amount: f32,
    pub attacker: Option<Entity>,
    /// Velocity impulse in blocks per simulation tick.
    pub impulse: Vec3,
    pub minimum_health: f32,
}

impl DamageRequest {
    pub fn new(target: Entity, source: &'static DamageType, amount: f32) -> Self {
        Self {
            target,
            source,
            amount,
            attacker: None,
            impulse: Vec3::ZERO,
            minimum_health: 0.0,
        }
    }
}

#[derive(Message, Clone, Copy)]
pub struct DamageApplied {
    pub target: Entity,
    pub attacker: Option<Entity>,
    pub source: &'static DamageType,
    pub lost_health: f32,
    pub fatal: bool,
}

pub(super) fn equipment(inventory: &PlayerInventory) -> ArmorStats {
    let mut total = ArmorStats::default();
    for index in ARMOR_START..ARMOR_START + 4 {
        if let Some(stack) = inventory.slot(index) {
            let armor = super::catalog::armor(stack.item());
            total.protection += armor.protection;
            total.toughness += armor.toughness;
            total.knockback_resistance += armor.knockback_resistance;
        }
    }
    total.knockback_resistance = total.knockback_resistance.clamp(0.0, 1.0);
    total
}

pub(super) fn refresh_equipment(
    mut actors: Query<(Ref<PlayerInventory>, &mut ArmorStats), With<crate::simulation::Simulating>>,
) {
    for (inventory, mut armor) in &mut actors {
        if inventory.is_changed() || armor.is_added() {
            armor.set_if_neq(equipment(&inventory));
        }
    }
}

pub(super) fn advance_windows(
    mut actors: Query<&mut DamageState, With<crate::simulation::Simulating>>,
) {
    actors.par_iter_mut().for_each(|mut state| {
        if state.invulnerable_ticks > 0 {
            state.invulnerable_ticks -= 1;
        }
        if state.flash_ticks > 0 {
            state.flash_ticks -= 1;
        }
    });
}

#[allow(clippy::type_complexity)]
pub(super) fn resolve(
    mut requests: MessageReader<DamageRequest>,
    mut applied: MessageWriter<DamageApplied>,
    mut actors: Query<
        (
            &mut Health,
            &mut DamageState,
            &ActiveEffects,
            &mut ArmorStats,
            Option<&mut PlayerInventory>,
            Option<&mut CharacterMotion>,
            Option<&mut Nutrition>,
            Option<&mut CharacterMovement>,
            Option<&mut UsingItem>,
        ),
        With<crate::simulation::Simulating>,
    >,
) {
    for request in requests.read() {
        if !request.amount.is_finite()
            || request.amount <= 0.0
            || !request.impulse.is_finite()
            || !request.minimum_health.is_finite()
        {
            continue;
        }
        let melee = request.source.id == DamageType::PLAYER_ATTACK.id
            || request.source.id == DamageType::MOB_ATTACK.id;
        let attacker_bonus = if melee {
            request
                .attacker
                .and_then(|entity| actors.get(entity).ok())
                .map_or(0.0, |(_, _, effects, ..)| {
                    3.0 * effects.level(E::STRENGTH.id) as f32
                        - 4.0 * effects.level(E::WEAKNESS.id) as f32
                })
        } else {
            0.0
        };
        let requested_amount = (request.amount + attacker_bonus).max(0.0);
        if requested_amount == 0.0 {
            continue;
        }
        let Ok((
            mut health,
            mut state,
            effects,
            mut armor,
            inventory,
            motion,
            nutrition,
            movement,
            using,
        )) = actors.get_mut(request.target)
        else {
            continue;
        };
        if !health.alive() || health.current <= request.minimum_health {
            continue;
        }
        let source = request.source;
        if source.has_tag(&tag::DamageType::MINECRAFT_IS_FIRE)
            && effects.level(E::FIRE_RESISTANCE.id) > 0
        {
            continue;
        }
        let bypass_cooldown = source.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_COOLDOWN)
            || source.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_INVULNERABILITY);
        let fresh = state.invulnerable_ticks <= 10 || bypass_cooldown;
        let mut amount = requested_amount;
        if !fresh {
            if amount <= state.last_amount {
                continue;
            }
            amount -= state.last_amount;
        }
        state.last_amount = requested_amount;
        if fresh {
            state.invulnerable_ticks = 20;
        }
        let armor_applies = !source.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_ARMOR);
        let raw = amount;
        if armor_applies {
            let reduction = (armor.protection - amount / (2.0 + armor.toughness / 4.0))
                .clamp(armor.protection * 0.2, 20.0);
            amount *= 1.0 - reduction / 25.0;
        }
        if !source.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_RESISTANCE)
            && !source.has_tag(&tag::DamageType::MINECRAFT_BYPASSES_EFFECTS)
        {
            amount *= (1.0 - effects.level(E::RESISTANCE.id) as f32 * 0.2).max(0.0);
        }
        if amount <= 0.0 {
            continue;
        }
        let absorbed = amount.min(health.absorption);
        health.absorption -= absorbed;
        let lost_health =
            (amount - absorbed).min((health.current - request.minimum_health.max(0.0)).max(0.0));
        health.current -= lost_health;
        state.flash_ticks = 10;
        state.source = source.id;
        state.direction = request.impulse.normalize_or_zero().to_array();
        if let Some(mut inventory) = inventory
            && armor_applies
        {
            PlayerInventory::edit(&mut inventory, |inventory| {
                for slot in ARMOR_START..ARMOR_START + 4 {
                    inventory.damage_tool(slot, (raw / 4.0).floor().max(1.0) as u32);
                }
            });
            armor.set_if_neq(equipment(&inventory));
        }
        if let Some(mut nutrition) = nutrition {
            nutrition.exhaust(source.exhaustion);
        }
        if let Some(mut motion) = motion {
            if fresh && !source.has_tag(&tag::DamageType::MINECRAFT_NO_KNOCKBACK) {
                let impulse = request.impulse * (1.0 - armor.knockback_resistance);
                if impulse != Vec3::ZERO {
                    motion.velocity.x *= 0.5;
                    motion.velocity.z *= 0.5;
                    motion.velocity += impulse.as_dvec3();
                }
            }
            if !health.alive() {
                motion.velocity = bevy::math::DVec3::ZERO;
            }
        }
        if !health.alive() {
            if let Some(mut movement) = movement {
                *movement = CharacterMovement::default();
            }
            if let Some(mut using) = using {
                using.0 = None;
            }
        }
        applied.write(DamageApplied {
            target: request.target,
            attacker: request.attacker,
            source,
            lost_health,
            fatal: !health.alive(),
        });
    }
}
