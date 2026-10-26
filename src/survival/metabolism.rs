use bevy::prelude::*;
use pumpkin_data::{damage::DamageType, effect::StatusEffect as E};

use super::placement::{HazardKind, hazard};
use super::{ActiveEffects, DamageRequest, Hazards, Health, Nutrition, SurvivalRules, UsingItem};
use crate::{
    character::{CharacterActivity, CharacterBody, CharacterMotion, CharacterState, Locomotion},
    voxel::{VoxelKind, VoxelWorld},
};

pub(super) fn locomotion(
    mut actors: Query<(
        &Health,
        &ActiveEffects,
        Option<&Nutrition>,
        Option<&UsingItem>,
        &mut Locomotion,
    )>,
) {
    actors
        .par_iter_mut()
        .for_each(|(health, effects, nutrition, using, mut locomotion)| {
            let mut next = Locomotion {
                speed: (1.0 + 0.2 * effects.level(E::SPEED.id) as f64)
                    * (1.0 - 0.15 * effects.level(E::SLOWNESS.id) as f64).max(0.0),
                jump_boost: effects.level(E::JUMP_BOOST.id) as f64 * 0.1,
                sprint_allowed: health.alive()
                    && nutrition.is_none_or(|nutrition| nutrition.food > 6)
                    && effects.level(E::BLINDNESS.id) == 0,
                slow_falling: effects.level(E::SLOW_FALLING.id) > 0,
            };
            if let Some(action) = using.and_then(|using| using.0) {
                next.speed *= super::catalog::consumable(action.stack.item())
                    .map_or(0.2, |profile| profile.speed);
                next.sprint_allowed = false;
            }
            locomotion.set_if_neq(next);
        });
}

pub(super) fn nutrition(
    rules: Res<SurvivalRules>,
    mut actors: Query<(
        Entity,
        &mut Health,
        &mut Nutrition,
        &CharacterActivity,
        &CharacterState,
    )>,
    mut damage: MessageWriter<DamageRequest>,
) {
    for (entity, mut health, mut nutrition, activity, state) in &mut actors {
        if !health.alive() || !activity.simulated {
            continue;
        }
        if activity.jumped {
            nutrition.exhaust(if activity.sprinting { 0.2 } else { 0.05 });
        }
        if state.in_water {
            nutrition.exhaust(activity.distance.hypot(activity.vertical) * 0.01);
        } else if state.on_ground && activity.sprinting {
            nutrition.exhaust(activity.distance * 0.1);
        }
        while nutrition.exhaustion >= 4.0 {
            nutrition.exhaustion -= 4.0;
            if nutrition.saturation > 0.0 {
                nutrition.saturation = (nutrition.saturation - 1.0).max(0.0);
            } else {
                nutrition.food = nutrition.food.saturating_sub(1);
            }
        }
        let needs_healing = health.current < health.maximum;
        let fast = rules.natural_regeneration
            && needs_healing
            && nutrition.food == 20
            && nutrition.saturation > 0.0;
        let regenerating = rules.natural_regeneration && needs_healing && nutrition.food >= 18;
        if fast || regenerating || nutrition.food == 0 {
            nutrition.recovery_ticks += 1;
            let period = if fast { 10 } else { 80 };
            if nutrition.recovery_ticks >= period {
                nutrition.recovery_ticks = 0;
                if fast {
                    let amount = nutrition.saturation.min(6.0);
                    health.heal(amount / 6.0);
                    nutrition.exhaust(amount);
                } else if regenerating {
                    health.heal(1.0);
                    nutrition.exhaust(6.0);
                } else {
                    let mut request = DamageRequest::new(entity, &DamageType::STARVE, 1.0);
                    request.minimum_health = rules.starvation_floor;
                    damage.write(request);
                }
            }
        } else if nutrition.recovery_ticks != 0 {
            nutrition.recovery_ticks = 0;
        }
    }
}

/// Hazard sampling is bounded by the character AABB and only reads loaded columns.
#[allow(clippy::type_complexity)]
pub(super) fn hazards(
    world: VoxelWorld,
    mut actors: Query<(
        Entity,
        &Health,
        &ActiveEffects,
        &CharacterMotion,
        &CharacterBody,
        &CharacterState,
        &CharacterActivity,
        &mut Hazards,
    )>,
    mut damage: MessageWriter<DamageRequest>,
) {
    for (entity, health, effects, motion, body, state, activity, mut hazards) in &mut actors {
        if !health.alive() || !activity.simulated {
            continue;
        }
        let position = motion.position.as_vec3();
        if !world.collision_ready(position) {
            continue;
        }
        let feet = position - Vec3::Y * body.origin_height;
        let eye = feet + Vec3::Y * state.pose.eye_height();
        if activity.vertical < 0.0 {
            hazards.fall_distance -= activity.vertical;
        }
        if state.in_water
            || state.in_lava
            || activity.climbing
            || effects.level(E::SLOW_FALLING.id) > 0
        {
            if hazards.fall_distance != 0.0 {
                hazards.fall_distance = 0.0;
            }
        } else if state.on_ground && hazards.fall_distance > 0.0 {
            let safe_distance = 3.0 + effects.level(E::JUMP_BOOST.id) as f32;
            let amount = (hazards.fall_distance - safe_distance).ceil();
            if amount > 0.0 {
                damage.write(DamageRequest::new(entity, &DamageType::FALL, amount));
            }
            hazards.fall_distance = 0.0;
        }
        if state.eyes_in_water && effects.level(E::WATER_BREATHING.id) == 0 {
            hazards.air_used += 1;
            if hazards.air_used >= 320 {
                hazards.air_used = 300;
                damage.write(DamageRequest::new(entity, &DamageType::DROWN, 2.0));
            }
        } else if hazards.air_used > 0 {
            hazards.air_used = hazards.air_used.saturating_sub(4);
        }
        hazards.contact_ticks = (hazards.contact_ticks + 1) % 10;
        let mut fire = false;
        let mut contact = None;
        let min = (feet - Vec3::new(body.half_width + 0.01, 0.01, body.half_width + 0.01))
            .floor()
            .as_ivec3();
        let max = (feet
            + Vec3::new(
                body.half_width + 0.01,
                state.pose.height() - 0.01,
                body.half_width + 0.01,
            ))
        .floor()
        .as_ivec3();
        world.visit_cells(min, max, |_, block| {
            let Some(block) = block else {
                return;
            };
            match hazard(block.kind()) {
                HazardKind::Fire => fire = true,
                HazardKind::Cactus => contact = Some((&DamageType::CACTUS, 1.0)),
                HazardKind::Berry if activity.distance > 0.001 => {
                    contact = Some((&DamageType::SWEET_BERRY_BUSH, 1.0))
                }
                HazardKind::HotFloor if !activity.jumped && activity.vertical.abs() < 0.01 => {
                    contact = Some((&DamageType::HOT_FLOOR, 1.0))
                }
                HazardKind::Campfire
                    if block.property(pumpkin_data::PropertyKey::Lit)
                        == Some(pumpkin_data::Property::Lit(true)) =>
                {
                    contact = Some((&DamageType::CAMPFIRE, 1.0));
                }
                _ => {}
            }
        });
        if state.in_water {
            if hazards.burning_ticks > 0 {
                hazards.burning_ticks = 0;
            }
        } else if state.in_lava || fire {
            hazards.burning_ticks = 160;
        }
        if hazards.burning_ticks > 0 {
            hazards.burning_ticks -= 1;
            if hazards.burning_ticks % 20 == 0 {
                damage.write(DamageRequest::new(entity, &DamageType::ON_FIRE, 1.0));
            }
        }
        if hazards.contact_ticks == 0 {
            if state.in_lava {
                damage.write(DamageRequest::new(entity, &DamageType::LAVA, 4.0));
            } else if fire {
                damage.write(DamageRequest::new(entity, &DamageType::IN_FIRE, 1.0));
            }
            if let Some((source, amount)) = contact {
                damage.write(DamageRequest::new(entity, source, amount));
            }
            if let Some(block) = world.state(eye.floor().as_ivec3()) {
                let local_eye = eye - eye.floor();
                if block.kind() != VoxelKind::Air
                    && block.collision_boxes(eye.floor().as_ivec3()).any(|shape| {
                        local_eye.cmpgt(shape.min).all() && local_eye.cmplt(shape.max).all()
                    })
                {
                    damage.write(DamageRequest::new(entity, &DamageType::IN_WALL, 1.0));
                }
            }
            if feet.y < -8.0 {
                damage.write(DamageRequest::new(entity, &DamageType::OUT_OF_WORLD, 4.0));
            }
        }
    }
}
