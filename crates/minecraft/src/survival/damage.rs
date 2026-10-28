use bevy::prelude::*;
use pumpkin_data::{
    damage::DamageType,
    effect::StatusEffect as E,
    tag::{self, Taggable},
};

use super::{ActiveEffects, ArmorStats, DamageState, Health};
use crate::inventory::{ARMOR_START, PlayerInventory};

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

/// Accepted impacts react at the command boundary after the ordered damage batch.
#[derive(EntityEvent, Clone, Copy)]
pub struct DamageApplied {
    #[event_target]
    pub target: Entity,
    pub attacker: Option<Entity>,
    pub source: &'static DamageType,
    pub lost_health: f32,
    /// Resolved knockback, after cooldown and equipment resistance.
    pub impulse: Vec3,
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
    mut commands: Commands,
    mut actors: Query<
        (
            &mut Health,
            &mut DamageState,
            &ActiveEffects,
            &mut ArmorStats,
            Option<&mut PlayerInventory>,
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
        let Ok((mut health, mut state, effects, mut armor, inventory)) =
            actors.get_mut(request.target)
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
        // Durability stays in the ordered transaction: breaking armor must affect
        // the next request in this same batch, including knockback resistance.
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
        let impulse = if fresh && !source.has_tag(&tag::DamageType::MINECRAFT_NO_KNOCKBACK) {
            request.impulse * (1.0 - armor.knockback_resistance)
        } else {
            Vec3::ZERO
        };
        commands.trigger(DamageApplied {
            target: request.target,
            attacker: request.attacker,
            source,
            lost_health,
            impulse,
            fatal: !health.alive(),
        });
    }
}

/// The survival integration owns knockback; the controller has no damage dependency.
pub(super) fn apply_knockback(
    impact: On<DamageApplied>,
    mut motion: Query<&mut crate::character::CharacterMotion>,
    mut movement: Query<&mut crate::character::CharacterMovement>,
) {
    if let Ok(mut motion) = motion.get_mut(impact.target) {
        if impact.fatal {
            motion.velocity = bevy::math::DVec3::ZERO;
        } else if impact.impulse != Vec3::ZERO {
            motion.velocity.x *= 0.5;
            motion.velocity.z *= 0.5;
            motion.velocity += impact.impulse.as_dvec3();
        }
    }
    if impact.fatal
        && let Ok(mut movement) = movement.get_mut(impact.target)
    {
        movement.set_if_neq(crate::character::CharacterMovement::default());
    }
}
