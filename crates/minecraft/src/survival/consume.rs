use crate::inventory::catalog::ConsumeAction;
use bevy::{ecs::query::QueryFilter, prelude::*};
use hollow_features::motion::FixedPosition;

use super::{
    ActiveEffects, Hazards, Health, ItemUse, Nutrition, SurvivalRandom, UseInput, UsingItem,
};
use crate::{
    audio::{Sound, SoundId},
    character::{CharacterBody, CharacterMotion, CharacterState},
    dropped_item::DroppedItemCommandsExt,
    inventory::{ItemStack, OFFHAND_SLOT, PlayerInventory},
    voxel::VoxelWorld,
};

/// Published after the inventory transaction; consumers run in the same fixed tick.
#[derive(Message, Clone, Copy)]
pub struct ItemConsumed {
    pub entity: Entity,
    pub stack: ItemStack,
}

pub struct ItemUsePlugin;
impl Plugin for ItemUsePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SurvivalRandom>()
            .add_message::<ItemConsumed>()
            .add_systems(
                FixedUpdate,
                (tick, apply_consumption)
                    .chain()
                    .in_set(super::SurvivalSystems::Consume),
            )
            .add_systems(
                FixedUpdate,
                damage_response.in_set(super::SurvivalSystems::Reactions),
            )
            .add_systems(
                OnExit(crate::world::WorldSessionPhase::Playing),
                cancel::<()>,
            );
    }
}

pub(crate) fn cancel<F: QueryFilter>(mut actors: Query<(&mut UsingItem, &mut UseInput), F>) {
    for (using, input) in &mut actors {
        using.map_unchanged(|using| &mut using.0).set_if_neq(None);
        input.map_unchanged(|input| &mut input.0).set_if_neq(None);
    }
}

/// The interaction pass chooses the hand after giving usable blocks priority.
pub(crate) fn choose_slot(inventory: &PlayerInventory, selected: usize) -> Option<usize> {
    [selected, OFFHAND_SLOT].into_iter().find(|&slot| {
        inventory
            .slot(slot)
            .is_some_and(|stack| super::catalog::consumable(stack.item()).is_some())
    })
}

fn safe_teleport(
    world: &VoxelWorld,
    position: &FixedPosition,
    body: &CharacterBody,
    state: &CharacterState,
    random: &mut SurvivalRandom,
    diameter: f32,
) -> Option<Vec3> {
    for _ in 0..16 {
        let candidate = position.current.as_vec3()
            + Vec3::new(
                (random.unit() - 0.5) * diameter,
                (random.unit() - 0.5) * diameter,
                (random.unit() - 0.5) * diameter,
            );
        if !world.collision_ready(candidate) {
            continue;
        }
        let mut cell = candidate.floor().as_ivec3();
        for _ in 0..32 {
            let Some(block) = world.state(cell) else {
                break;
            };
            if block.collision_boxes(cell).next().is_some() {
                if let Some(position) =
                    super::placement::on_support(world, cell, candidate.xz(), body, state.pose)
                {
                    return Some(position);
                }
                break;
            }
            cell.y -= 1;
            if cell.y < 0 {
                break;
            }
        }
    }
    None
}

/// Controllers supply an intent; item use does not depend on local input or a body.
fn tick(
    world: VoxelWorld,
    mut commands: Commands,
    mut consumed: MessageWriter<ItemConsumed>,
    mut actors: Query<
        (
            Entity,
            &mut UsingItem,
            &UseInput,
            &mut PlayerInventory,
            &Health,
            Option<&Nutrition>,
            &FixedPosition,
        ),
        With<crate::simulation::Simulating>,
    >,
) {
    for (entity, mut using, input, mut inventory, health, nutrition, position) in &mut actors {
        let consumable = input
            .0
            .filter(|_| health.alive() && world.collision_ready(position.current.as_vec3()))
            .and_then(|slot| {
                let stack = inventory.slot(slot)?;
                let profile = super::catalog::consumable(stack.item())?;
                profile
                    .food
                    .is_none_or(|food| {
                        nutrition
                            .is_some_and(|nutrition| food.can_always_eat || nutrition.food < 20)
                    })
                    .then_some((slot, stack, profile))
            });
        let Some((slot, stack, profile)) = consumable else {
            if using.0.is_some() {
                using.0 = None;
            }
            continue;
        };
        if using
            .0
            .is_none_or(|action| action.slot != slot || !action.stack.can_merge(stack))
        {
            using.0 = Some(ItemUse {
                slot,
                stack,
                elapsed: 0,
                duration: profile.duration,
                drinking: profile.drinking,
            });
        }
        let action = using.0.as_mut().unwrap();
        action.elapsed += 1;
        if action.elapsed % 8 == 1 {
            commands.trigger(Sound::event(if action.drinking {
                SoundId::EntityGenericDrink
            } else {
                SoundId::EntityGenericEat
            }));
        }
        if action.elapsed < action.duration {
            continue;
        }
        // Validation, consumption and remainder insertion share one inventory borrow.
        PlayerInventory::edit(&mut inventory, |inventory| {
            inventory.consume(slot, 1);
            if let Some(item) = profile.remainder {
                use crate::inventory::slots::Slots;
                let remainder = ItemStack::new(item, 1);
                if inventory.slot(slot).is_none() {
                    inventory.set(slot, Some(remainder));
                } else if let Some(left) = inventory.insert(remainder) {
                    commands.spawn_dropped_item(left, position.current.as_vec3(), Vec3::Y);
                }
            }
        });
        consumed.write(ItemConsumed {
            entity,
            stack: stack.with_count(1).expect("Consumed one item"),
        });
        using.0 = None;
    }
}

fn apply_consumption(
    mut consumed: MessageReader<ItemConsumed>,
    mut actors: Query<(&mut Health, &mut ActiveEffects, Option<&mut Nutrition>)>,
    mut teleports: Query<(
        &mut Transform,
        &mut CharacterMotion,
        &FixedPosition,
        &CharacterBody,
        &CharacterState,
        &mut Hazards,
    )>,
    world: VoxelWorld,
    mut random: ResMut<SurvivalRandom>,
    mut commands: Commands,
) {
    for consumed in consumed.read() {
        let Ok((mut health, mut effects, mut nutrition)) = actors.get_mut(consumed.entity) else {
            continue;
        };
        let profile =
            super::catalog::consumable(consumed.stack.item()).expect("Consumed item profile");
        let previous_health = *health;
        let next_health = health.bypass_change_detection();
        if let Some(food) = profile.food
            && let Some(nutrition) = nutrition.as_deref_mut()
        {
            nutrition.food = (nutrition.food as i32 + food.nutrition).clamp(0, 20) as u8;
            nutrition.saturation =
                (nutrition.saturation + food.saturation).clamp(0.0, nutrition.food as f32);
            commands.trigger(Sound::event(SoundId::EntityPlayerBurp));
        }
        for effect in &profile.actions {
            match effect {
                ConsumeAction::Apply(instances, probability) if random.unit() < *probability => {
                    for effect in instances {
                        super::effects::apply(next_health, &mut effects, *effect);
                    }
                }
                ConsumeAction::Clear => {
                    super::effects::remove(next_health, &mut effects, |_| false)
                }
                ConsumeAction::Remove(ids) => {
                    super::effects::remove(next_health, &mut effects, |effect| {
                        !ids.contains(&effect.id)
                    })
                }
                ConsumeAction::Teleport(diameter) => {
                    if let Ok((mut transform, mut motion, position, body, character, mut hazards)) =
                        teleports.get_mut(consumed.entity)
                        && let Some(position) =
                            safe_teleport(&world, position, body, character, &mut random, *diameter)
                    {
                        transform.translation = position;
                        motion.velocity = bevy::math::DVec3::ZERO;
                        hazards.fall_distance = 0.0;
                        commands.trigger(Sound::event(SoundId::ItemChorusFruitTeleport));
                    }
                }
                ConsumeAction::Sound => {
                    commands.trigger(Sound::event(SoundId::EntityPlayerBurp));
                }
                _ => {}
            }
        }
        for effect in profile.effects(consumed.stack.contents()) {
            super::effects::apply(next_health, &mut effects, effect);
        }
        if *next_health != previous_health {
            health.set_changed();
        }
    }
}

pub(super) fn damage_response(
    mut impacts: MessageReader<super::DamageApplied>,
    mut actors: Query<&mut UsingItem>,
) {
    for impact in impacts.read().filter(|impact| impact.fatal) {
        if let Ok(using) = actors.get_mut(impact.target) {
            using.map_unchanged(|using| &mut using.0).set_if_neq(None);
        }
    }
}
