use super::*;
use crate::{
    character::{CharacterBody, CharacterState},
    simulation::{SimulatedIn, Simulating},
    survival::{DamageApplied, DamageRequest, Health},
    voxel::VoxelWorld,
};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_features::motion::FixedPosition;

#[derive(Component)]
pub(crate) struct Dying(pub u8);

/// Player input borrows this adapter; the damage resolver remains the authority.
#[derive(SystemParam)]
pub struct CreatureCombat<'w, 's> {
    columns: Res<'w, crate::block_behavior::containers::GameplayColumns>,
    members: Query<'w, 's, &'static crate::simulation::SimulatedEntities>,
    catalog: Res<'w, CreatureCatalog>,
    actors: Query<
        'w,
        's,
        (
            Entity,
            &'static FixedPosition,
            &'static CharacterBody,
            &'static CharacterState,
            &'static Health,
        ),
        (With<Creature>, With<Simulating>),
    >,
    damage: MessageWriter<'w, DamageRequest>,
    next_attack: Local<'s, u64>,
}
impl CreatureCombat<'_, '_> {
    /// Returns whether an animal intercepts the held mining input, even on cooldown.
    pub(crate) fn attack(
        &mut self,
        actor: Entity,
        origin: Vec3,
        look: Vec3,
        tick: u64,
        world: &VoxelWorld,
        inventory: &mut crate::inventory::PlayerInventory,
        selection: &crate::inventory::HotbarSelection,
    ) -> bool {
        let Ok(direction) = Dir3::new(look) else {
            return false;
        };
        let mut ready = true;
        let obstruction = hollow_engine::raycast::raycast(
            origin,
            direction,
            3.0,
            1,
            |p| {
                let state = world.state(p);
                ready &= state.is_some();
                state
            },
            |p, s| (!s.collision_empty()).then(|| s.collision_boxes(p)),
        );
        if !ready {
            return false;
        }
        let reach = obstruction.map_or(3.0, |hit| hit.location.distance(origin));
        let end = origin + direction.as_vec3() * reach;
        let padding = Vec3::splat(self.catalog.maximum_half_width);
        let min = crate::voxel::chunk_at(origin.min(end) - padding);
        let max = crate::voxel::chunk_at(origin.max(end) + padding);
        let mut closest: Option<(Entity, Vec3, f32)> = None;
        for x in min.x..=max.x {
            for z in min.y..=max.y {
                let Some(owner) = self.columns.get(&IVec2::new(x, z)) else {
                    continue;
                };
                let Ok(members) = self.members.get(*owner) else {
                    continue;
                };
                for entity in members.iter() {
                    let Ok((entity, position, body, state, health)) = self.actors.get(entity)
                    else {
                        continue;
                    };
                    if !health.alive() {
                        continue;
                    }
                    let bounds = body.aabb(position.current, state.pose);
                    let shape = hollow_core::shape::BlockBox {
                        min: bounds.min.as_vec3(),
                        max: bounds.max.as_vec3(),
                    };
                    if let Some((distance, _)) = shape.ray(origin, direction.as_vec3())
                        && distance < reach
                        && closest.as_ref().is_none_or(|other| distance < other.2)
                    {
                        closest = Some((entity, position.current.as_vec3(), distance));
                    }
                }
            }
        }
        let Some((target, position, _)) = closest else {
            return false;
        };
        // Reset on world-clock rewind when a different session is opened.
        if *self.next_attack > tick + 10 {
            *self.next_attack = 0;
        }
        if tick >= *self.next_attack {
            *self.next_attack = tick + 10;
            let (damage, wear) = selection.stack(inventory).map_or((1.0, 0), |stack| {
                let properties = crate::inventory::catalog::properties(stack.item());
                (properties.attack_damage, properties.attack_wear)
            });
            let mut request = DamageRequest::new(
                target,
                &pumpkin_data::damage::DamageType::PLAYER_ATTACK,
                damage,
            );
            request.attacker = Some(actor);
            request.impulse =
                (position - origin).with_y(0.0).normalize_or_zero() * 0.4 + Vec3::Y * 0.4;
            self.damage.write(request);
            if wear > 0 {
                inventory.damage_tool(selection.0, wear);
            }
        }
        true
    }
}

pub struct CreatureCombatPlugin;
impl Plugin for CreatureCombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(death)
            .add_systems(FixedUpdate, expire.in_set(CreatureSystems::Finish));
    }
}
fn death(
    impact: On<DamageApplied>,
    actors: Query<(&Creature, &FixedPosition, Option<&SimulatedIn>, Has<Dying>)>,
    catalog: Res<CreatureCatalog>,
    mut random: ResMut<CreatureRandom>,
    mut columns: Query<&mut crate::block_behavior::containers::GameplayColumn>,
    mut commands: Commands,
) {
    let Ok((creature, position, owner, dying)) = actors.get(impact.target) else {
        return;
    };
    let Some(profile) = catalog.get(creature.0) else {
        return;
    };
    if dying {
        return;
    }
    let sound = if impact.fatal {
        profile.sounds.death
    } else {
        profile.sounds.hurt
    };
    commands.trigger(crate::audio::Sound::at(sound, position.current.as_vec3()));
    if !impact.fatal {
        return;
    }
    commands.entity(impact.target).insert(Dying(10));
    if let Some(owner) = owner
        && let Ok(mut column) = columns.get_mut(owner.0)
    {
        column.revision += 1;
    }
    use crate::dropped_item::DroppedItemCommandsExt;
    for &(item, minimum, maximum) in profile.loot {
        let count = minimum + random.range(u32::from(maximum - minimum) + 1) as u8;
        if count == 0 {
            continue;
        }
        if let Some(item) = crate::inventory::ItemKind::from_id(item) {
            commands.spawn_dropped_item(
                crate::inventory::ItemStack::new(item, count),
                position.current.as_vec3(),
                Vec3::Y * 1.2,
            );
        }
    }
}
fn expire(mut commands: Commands, mut actors: Query<(Entity, &mut Dying), With<Simulating>>) {
    for (entity, mut dying) in &mut actors {
        dying.0 = dying.0.saturating_sub(1);
        if dying.0 == 0 {
            commands.entity(entity).despawn();
        }
    }
}
