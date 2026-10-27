use bevy::prelude::*;
use pumpkin_data::{damage::DamageType, effect::StatusEffect as E};

use super::{ActiveEffect, ActiveEffects, DamageRequest, Health, Nutrition};

/// Reject unsupported per-stack effects instead of consuming them as no-ops.
pub(crate) fn supported(id: u8) -> bool {
    [
        E::ABSORPTION.id,
        E::REGENERATION.id,
        E::POISON.id,
        E::WITHER.id,
        E::HUNGER.id,
        E::RESISTANCE.id,
        E::FIRE_RESISTANCE.id,
        E::NAUSEA.id,
        E::BLINDNESS.id,
        E::JUMP_BOOST.id,
        E::SPEED.id,
        E::SLOWNESS.id,
        E::WATER_BREATHING.id,
        E::INSTANT_HEALTH.id,
        E::INSTANT_DAMAGE.id,
        E::SATURATION.id,
        E::HEALTH_BOOST.id,
        E::SLOW_FALLING.id,
        E::WEAKNESS.id,
        E::STRENGTH.id,
        E::HASTE.id,
        E::MINING_FATIGUE.id,
        E::NIGHT_VISION.id,
    ]
    .contains(&id)
}

pub(super) fn apply(health: &mut Health, effects: &mut ActiveEffects, effect: ActiveEffect) {
    let ActiveEffect {
        id,
        amplifier,
        ticks,
        show_icon,
    } = effect;
    if !supported(id) || ticks == 0 || !health.alive() {
        return;
    }
    let amplifier = amplifier.min(4);
    if let Some(current) = effects.0.iter_mut().find(|effect| effect.id == id) {
        if current.amplifier > amplifier {
            return;
        }
        if id == E::ABSORPTION.id {
            health.absorption = health.absorption.max(4.0 * (amplifier + 1) as f32);
        }
        if id == E::HEALTH_BOOST.id {
            health.maximum += 4.0 * amplifier.saturating_sub(current.amplifier) as f32;
        }
        if current.amplifier < amplifier {
            current.ticks = ticks;
        } else {
            current.ticks = current.ticks.max(ticks);
        }
        current.amplifier = amplifier;
        current.show_icon = show_icon;
        return;
    }
    if id == E::ABSORPTION.id {
        health.absorption += 4.0 * (amplifier + 1) as f32;
    }
    if id == E::HEALTH_BOOST.id {
        health.maximum += 4.0 * (amplifier + 1) as f32;
    }
    effects.0.push(ActiveEffect {
        id,
        amplifier,
        ticks: ticks.min(1_728_000),
        show_icon,
    });
}

pub(super) fn remove(
    health: &mut Health,
    effects: &mut ActiveEffects,
    keep: impl Fn(&ActiveEffect) -> bool,
) {
    effects.0.retain(|effect| {
        if keep(effect) {
            return true;
        }
        if effect.id == E::ABSORPTION.id {
            health.absorption = 0.0;
        }
        if effect.id == E::HEALTH_BOOST.id {
            health.maximum = (health.maximum - 4.0 * (effect.amplifier + 1) as f32).max(1.0);
            health.current = health.current.min(health.maximum);
        }
        false
    });
}

pub(super) fn tick(
    mut actors: Query<(
        Entity,
        &mut Health,
        &mut ActiveEffects,
        Option<&mut Nutrition>,
    )>,
    mut damage: MessageWriter<DamageRequest>,
) {
    for (entity, mut health, mut effects, mut nutrition) in &mut actors {
        if !health.alive() || effects.0.is_empty() {
            continue;
        }
        let mut expired = false;
        for effect in &mut effects.0 {
            let level = effect.amplifier + 1;
            let cadence = if effect.id == E::REGENERATION.id {
                50
            } else if effect.id == E::POISON.id {
                25
            } else if effect.id == E::WITHER.id {
                40
            } else {
                1
            };
            if effect.ticks % (cadence >> effect.amplifier).max(1) == 0 {
                match effect.id {
                    id if id == E::REGENERATION.id => health.heal(1.0),
                    id if id == E::POISON.id || id == E::WITHER.id => {
                        let mut request = DamageRequest::new(
                            entity,
                            if id == E::POISON.id {
                                &DamageType::MAGIC
                            } else {
                                &DamageType::WITHER
                            },
                            1.0,
                        );
                        if id == E::POISON.id {
                            request.minimum_health = 1.0;
                        }
                        damage.write(request);
                    }
                    id if id == E::INSTANT_HEALTH.id => {
                        health.heal((4u32 << effect.amplifier) as f32)
                    }
                    id if id == E::INSTANT_DAMAGE.id => {
                        damage.write(DamageRequest::new(
                            entity,
                            &DamageType::MAGIC,
                            (6u32 << effect.amplifier) as f32,
                        ));
                    }
                    id if id == E::HUNGER.id => {
                        if let Some(nutrition) = nutrition.as_deref_mut() {
                            nutrition.exhaust(0.005 * level as f32);
                        }
                    }
                    id if id == E::SATURATION.id => {
                        if let Some(nutrition) = nutrition.as_deref_mut() {
                            nutrition.food = (nutrition.food + level).min(20);
                            nutrition.saturation = (nutrition.saturation + 2.0 * level as f32)
                                .min(nutrition.food as f32);
                        }
                    }
                    _ => {}
                }
            }
            effect.ticks -= 1;
            expired |= effect.ticks == 0;
        }
        // Expiration must undo attribute contributions exactly once.
        if expired {
            remove(&mut health, &mut effects, |effect| effect.ticks != 0);
        }
    }
}
