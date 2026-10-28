use super::*;
use crate::{
    block_behavior::containers::GameplayColumn,
    character::{CharacterMotion, CharacterState},
    simulation::SimulatedIn,
    survival::{ActiveEffects, DamageState, Hazards, Health},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_features::motion::FixedPosition;

/// Dormant mobile data remains owned by the same gameplay column as block state.
#[derive(Default)]
pub struct ColumnActors {
    pub(crate) drops: Vec<crate::world::SavedDrop>,
    pub(crate) creatures: Vec<SavedCreature>,
}
impl ColumnActors {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.drops.capacity() * std::mem::size_of::<crate::world::SavedDrop>()
            + self.creatures.capacity() * std::mem::size_of::<SavedCreature>()
            + self
                .creatures
                .iter()
                .map(|c| {
                    c.effects.0.capacity() * std::mem::size_of::<crate::survival::ActiveEffect>()
                })
                .sum::<usize>()
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedCreature {
    pub(crate) species: SpeciesId,
    pub(crate) source: SpawnSource,
    pub(crate) position: [f64; 3],
    pub(crate) velocity: [f64; 3],
    pub(crate) rotation: [f32; 4],
    pub(crate) health: Health,
    pub(crate) effects: ActiveEffects,
    pub(crate) damage: DamageState,
    pub(crate) hazards: Hazards,
}
impl SavedCreature {
    pub(crate) fn validate(&self, chunk: IVec2) -> std::io::Result<()> {
        let rotation = Quat::from_array(self.rotation);
        let valid = self
            .position
            .iter()
            .chain(self.velocity.iter())
            .all(|v| v.is_finite())
            && self.position.iter().all(|v| v.abs() <= 30_000_000.0)
            && (-8.0..=512.0).contains(&self.position[1])
            && crate::voxel::chunk_at(bevy::math::DVec3::from_array(self.position).as_vec3())
                == chunk
            && rotation.is_finite()
            && (0.99..=1.01).contains(&rotation.length_squared())
            && EntityType::from_raw(self.species.0).is_some()
            && self.health.alive()
            && crate::survival::validate_living(
                &self.health,
                &self.effects,
                &self.damage,
                &self.hazards,
            );
        if valid {
            Ok(())
        } else {
            Err(std::io::Error::other("Invalid saved creature."))
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SavedSpawner {
    pub position: [i32; 3],
    pub species: SpeciesId,
    pub delay: u16,
}

#[derive(SystemParam)]
pub(crate) struct CreatureSnapshots<'w, 's> {
    actors: Query<
        'w,
        's,
        (
            &'static Creature,
            &'static SpawnSource,
            &'static FixedPosition,
            &'static CharacterMotion,
            &'static Transform,
            &'static Health,
            &'static ActiveEffects,
            &'static DamageState,
            &'static Hazards,
        ),
    >,
    spawners: Query<
        'w,
        's,
        (
            &'static crate::block_behavior::containers::BlockEntity,
            &'static CreatureSpawner,
        ),
    >,
}
impl CreatureSnapshots<'_, '_> {
    pub(crate) fn contains(&self, entity: Entity) -> bool {
        self.actors
            .get(entity)
            .is_ok_and(|(.., health, _, _, _)| health.alive())
    }
    pub(crate) fn capture(&self, entity: Entity) -> Option<SavedCreature> {
        let (creature, source, position, motion, transform, health, effects, damage, hazards) =
            self.actors.get(entity).ok()?;
        if !health.alive() {
            return None;
        }
        Some(SavedCreature {
            species: creature.0,
            source: *source,
            position: position.current.to_array(),
            velocity: motion.velocity.to_array(),
            rotation: transform.rotation.to_array(),
            health: *health,
            effects: effects.clone(),
            damage: damage.clone(),
            hazards: hazards.clone(),
        })
    }
    pub(crate) fn spawner(&self, entity: Entity) -> Option<SavedSpawner> {
        let (block, spawner) = self.spawners.get(entity).ok()?;
        Some(SavedSpawner {
            position: block.position.to_array(),
            species: spawner.species,
            delay: spawner.delay,
        })
    }
    pub(crate) fn retirement_bytes(
        &self,
        mobiles: Option<&crate::simulation::SimulatedEntities>,
    ) -> usize {
        let bytes = mobiles
            .into_iter()
            .flat_map(|m| m.iter())
            .map(|entity| self.bytes(entity))
            .sum::<usize>();
        // Capture temporarily overlaps live state. Include Vec growth and its
        // initial four-record allocation rather than charging only occupied slots.
        if bytes == 0 {
            0
        } else {
            bytes * 2 + 4 * std::mem::size_of::<SavedCreature>()
        }
    }
    pub(crate) fn bytes(&self, entity: Entity) -> usize {
        self.actors
            .get(entity)
            .map_or(0, |(_, _, _, _, _, _, effects, _, _)| {
                std::mem::size_of::<SavedCreature>()
                    + effects.0.len() * std::mem::size_of::<crate::survival::ActiveEffect>()
            })
    }
}

impl Plugin for CreaturePersistencePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, track_changes.before(crate::world::WorldSaveCapture));
    }
}
fn track_changes(
    creatures: Query<
        &SimulatedIn,
        (
            With<Creature>,
            Or<(
                Changed<FixedPosition>,
                Changed<Health>,
                Changed<ActiveEffects>,
                Changed<DamageState>,
                Changed<Hazards>,
            )>,
        ),
    >,
    spawners: Query<&crate::block_behavior::containers::BlockOf, Changed<CreatureSpawner>>,
    mut columns: Query<&mut GameplayColumn>,
    mut changed_columns: Local<Vec<Entity>>,
) {
    changed_columns.clear();
    for owner in &creatures {
        changed_columns.push(owner.0);
    }
    for owner in &spawners {
        changed_columns.push(owner.0);
    }
    changed_columns.sort_unstable();
    changed_columns.dedup();
    for owner in changed_columns.iter().copied() {
        if let Ok(mut column) = columns.get_mut(owner) {
            column.revision += 1;
        }
    }
}

pub(crate) fn restore(
    commands: &mut Commands,
    saved: SavedCreature,
    catalog: &CreatureCatalog,
    owner: Entity,
) {
    let Some(profile) = catalog.get(saved.species) else {
        return;
    };
    let position = bevy::math::DVec3::from_array(saved.position);
    let entity = super::spawning::spawn(commands, profile, position, saved.source, owner);
    let mut motion = CharacterMotion::default();
    motion.velocity = bevy::math::DVec3::from_array(saved.velocity);
    let mut fixed = FixedPosition::new(position.as_vec3());
    fixed.current = position;
    fixed.previous = position;
    commands.entity(entity).insert((
        fixed,
        motion,
        Transform::from_translation(position.as_vec3())
            .with_rotation(Quat::from_array(saved.rotation)),
        saved.health,
        saved.effects,
        saved.damage,
        saved.hazards,
        CharacterState::default(),
    ));
}
