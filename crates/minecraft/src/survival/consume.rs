use crate::inventory::catalog::ConsumeAction;
use bevy::prelude::*;
use hollow_features::motion::FixedPosition;

use super::{
    ActiveEffects, Hazards, Health, ItemUse, Nutrition, SurvivalRandom, UseInput, UsingItem,
};
use crate::{
    character::{CharacterBody, CharacterMotion, CharacterState},
    dropped_item::DroppedItemCommandsExt,
    inventory::{HotbarSelection, InventoryState, ItemStack, OFFHAND_SLOT, PlayerInventory},
    voxel::VoxelWorld,
};

pub(super) fn cancel(mut actors: Query<(&mut UsingItem, &mut UseInput)>) {
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

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(super) fn tick(
    mouse: Res<ButtonInput<MouseButton>>,
    state: Res<State<InventoryState>>,
    panel: Option<Res<State<crate::devtools::DevPanelState>>>,
    world: VoxelWorld,
    mut random: ResMut<SurvivalRandom>,
    mut commands: Commands,
    mut sounds: MessageWriter<super::presentation::SoundCue>,
    mut actors: Query<
        (
            &mut UsingItem,
            &UseInput,
            &HotbarSelection,
            &mut PlayerInventory,
            &mut Health,
            &mut Nutrition,
            &mut ActiveEffects,
            &mut Transform,
            &mut CharacterMotion,
            &FixedPosition,
            &CharacterBody,
            &CharacterState,
            &mut Hazards,
        ),
        With<crate::simulation::Simulating>,
    >,
) {
    let input_allowed = *state.get() == InventoryState::Closed
        && mouse.pressed(MouseButton::Right)
        && panel
            .as_ref()
            .is_none_or(|panel| *panel.get() == crate::devtools::DevPanelState::Closed);
    for (
        mut using,
        input,
        selection,
        mut inventory,
        mut health,
        mut nutrition,
        mut effects,
        mut transform,
        mut motion,
        position,
        body,
        character,
        mut hazards,
    ) in &mut actors
    {
        let consumable = input
            .0
            .filter(|&slot| {
                input_allowed
                    && health.alive()
                    && (slot == selection.0 || slot == OFFHAND_SLOT)
                    && world.collision_ready(position.current.as_vec3())
            })
            .and_then(|slot| {
                let stack = inventory.slot(slot)?;
                let profile = super::catalog::consumable(stack.item())?;
                profile
                    .food
                    .is_none_or(|food| food.can_always_eat || nutrition.food < 20)
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
            sounds.write(if action.drinking {
                super::presentation::SoundCue::Drink
            } else {
                super::presentation::SoundCue::Eat
            });
        }
        if action.elapsed < action.duration {
            continue;
        }
        // The same owner is borrowed through the whole transaction. No other action
        // can consume the stack between validation and the application of its effects.
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
        if let Some(food) = profile.food {
            nutrition.food = (nutrition.food as i32 + food.nutrition).clamp(0, 20) as u8;
            nutrition.saturation =
                (nutrition.saturation + food.saturation).clamp(0.0, nutrition.food as f32);
            sounds.write(super::presentation::SoundCue::Burp);
        }
        for effect in &profile.actions {
            match effect {
                ConsumeAction::Apply(instances, probability) if random.unit() < *probability => {
                    for effect in instances {
                        super::effects::apply(&mut health, &mut effects, *effect);
                    }
                }
                ConsumeAction::Clear => {
                    super::effects::remove(&mut health, &mut effects, |_| false)
                }
                ConsumeAction::Remove(ids) => {
                    super::effects::remove(&mut health, &mut effects, |effect| {
                        !ids.contains(&effect.id)
                    })
                }
                ConsumeAction::Teleport(diameter) => {
                    if let Some(position) =
                        safe_teleport(&world, position, body, character, &mut random, *diameter)
                    {
                        transform.translation = position;
                        motion.velocity = bevy::math::DVec3::ZERO;
                        hazards.fall_distance = 0.0;
                        sounds.write(super::presentation::SoundCue::Teleport);
                    }
                }
                ConsumeAction::Sound => {
                    sounds.write(super::presentation::SoundCue::Burp);
                }
                _ => {}
            }
        }
        for effect in profile.effects(stack.contents()) {
            super::effects::apply(&mut health, &mut effects, effect);
        }
        using.0 = None;
    }
}

pub(super) fn damage_response(
    mut impacts: MessageReader<super::DamageApplied>,
    mut actors: Query<&mut UsingItem>,
) {
    for impact in impacts.read().filter(|impact| impact.fatal) {
        if let Ok(mut using) = actors.get_mut(impact.target) {
            using.0 = None;
        }
    }
}
