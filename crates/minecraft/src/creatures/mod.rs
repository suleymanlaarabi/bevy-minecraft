//! Creature content composes independent actor capabilities over character physics.
mod behavior;
mod combat;
mod navigation;
pub(crate) mod persistence;
mod presentation;
pub(crate) mod spawning;

pub use combat::{CreatureCombat, CreatureCombatPlugin};
pub(crate) use navigation::PathPool;
pub use navigation::VoxelNavigator;
pub use persistence::{ColumnActors, SavedCreature};
pub(crate) use persistence::{CreatureSnapshots, SavedSpawner};
pub use spawning::{
    CreaturePopulationLimit, CreatureSpawner, SpawnCreature, SpawnCreaturePopulation, SpawnSource,
};

use bevy::prelude::*;
use hollow_features::actors::{LocomotionAnimation, ThreatMemory};
use pumpkin_data::{attributes::Attributes, entity::EntityType};

use crate::{
    character::{CharacterBody, CharacterShape},
    survival::Living,
};

/// The native registry ID is the only species identity in gameplay and saves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpeciesId(pub u16);

#[derive(Clone, Copy)]
pub enum QuadrupedRig {
    Cow,
    Pig,
}

pub struct CreatureProfile {
    pub native: &'static EntityType,
    /// Runs once at creation; no behavior graph or dispatch is stored per actor.
    pub compose: fn(&mut bevy::ecs::system::EntityCommands),
    pub body: CharacterBody,
    pub rig: QuadrupedRig,
    pub loot: &'static [(u16, u8, u8)],
    pub sounds: CreatureSounds,
    pub dimension: hollow_worldgen::Dimension,
}

#[derive(Clone, Copy)]
pub struct CreatureSounds {
    pub ambient: crate::audio::SoundId,
    pub hurt: crate::audio::SoundId,
    pub death: crate::audio::SoundId,
    pub step: crate::audio::SoundId,
}

static COW_SHAPE: CharacterShape = animal_shape(&EntityType::COW);
static PIG_SHAPE: CharacterShape = animal_shape(&EntityType::PIG);

const fn attribute(native: &EntityType, key: &Attributes, fallback: f64) -> f64 {
    let mut i = 0;
    while i < native.attributes.len() {
        if native.attributes[i].0.id == key.id {
            return native.attributes[i].1;
        }
        i += 1;
    }
    fallback
}

const fn animal_shape(native: &EntityType) -> CharacterShape {
    CharacterShape {
        half_width: native.dimension[0] * 0.5,
        origin_height: native.dimension[1] * 0.5,
        heights: [native.dimension[1]; 3],
        eyes: [native.eye_height; 3],
        step_height: attribute(native, &Attributes::STEP_HEIGHT, 0.6),
        jump_velocity: attribute(native, &Attributes::JUMP_STRENGTH, 0.42),
        // Idle wandering uses a quarter of the native movement attribute.
        speed: attribute(native, &Attributes::MOVEMENT_SPEED, 0.2) * 2.5,
    }
}

static COW: CreatureProfile = CreatureProfile {
    native: &EntityType::COW,
    compose: passive_quadruped,
    body: CharacterBody(&COW_SHAPE),
    rig: QuadrupedRig::Cow,
    loot: &[
        (pumpkin_data::item::Item::BEEF.id, 1, 3),
        (pumpkin_data::item::Item::LEATHER.id, 0, 2),
    ],
    sounds: CreatureSounds {
        ambient: crate::audio::SoundId::EntityCowAmbient,
        hurt: crate::audio::SoundId::EntityCowHurt,
        death: crate::audio::SoundId::EntityCowDeath,
        step: crate::audio::SoundId::EntityCowStep,
    },
    dimension: hollow_worldgen::Dimension::Overworld,
};
static PIG: CreatureProfile = CreatureProfile {
    native: &EntityType::PIG,
    compose: passive_quadruped,
    body: CharacterBody(&PIG_SHAPE),
    rig: QuadrupedRig::Pig,
    loot: &[(pumpkin_data::item::Item::PORKCHOP.id, 1, 3)],
    sounds: CreatureSounds {
        ambient: crate::audio::SoundId::EntityPigAmbient,
        hurt: crate::audio::SoundId::EntityPigHurt,
        death: crate::audio::SoundId::EntityPigDeath,
        step: crate::audio::SoundId::EntityPigStep,
    },
    dimension: hollow_worldgen::Dimension::Overworld,
};

#[derive(Resource)]
pub struct CreatureCatalog {
    // The native domain is small and bounded; one pointer per registry entry.
    profiles: Box<[Option<&'static CreatureProfile>]>,
    maximum_half_width: f32,
    pub(crate) biomes: Box<[Vec<SpawnEntry>]>,
}
#[derive(Clone, Copy)]
pub(crate) struct SpawnEntry {
    species: SpeciesId,
    minimum: u8,
    maximum: u8,
}
impl Default for CreatureCatalog {
    fn default() -> Self {
        let size = EntityType::ALL
            .iter()
            .map(|e| e.id as usize)
            .max()
            .unwrap_or(0)
            + 1;
        Self {
            profiles: vec![None; size].into_boxed_slice(),
            maximum_half_width: 0.0,
            biomes: (0..256).map(|_| Vec::new()).collect(),
        }
    }
}
impl CreatureCatalog {
    /// Register Rust content before app startup; duplicate/native IDs are rejected.
    pub fn register(&mut self, profile: &'static CreatureProfile) {
        assert!(
            profile.body.half_width.is_finite()
                && profile.body.half_width > 0.0
                && profile.body.origin_height.is_finite(),
            "Invalid creature footprint"
        );
        assert!(
            profile
                .body
                .heights
                .iter()
                .zip(profile.body.eyes)
                .all(|(height, eye)| height.is_finite()
                    && *height > 0.0
                    && eye.is_finite()
                    && eye >= 0.0
                    && eye <= *height),
            "Invalid creature poses"
        );
        assert!(
            profile.body.speed.is_finite()
                && profile.body.speed >= 0.0
                && profile.body.step_height.is_finite()
                && profile.body.step_height >= 0.0
                && profile.body.jump_velocity.is_finite()
                && profile.body.jump_velocity >= 0.0,
            "Invalid creature movement"
        );
        assert!(
            profile
                .loot
                .iter()
                .all(|(item, minimum, maximum)| minimum <= maximum
                    && crate::inventory::ItemKind::from_id(*item).is_some_and(|item| {
                        let limit = item
                            .component::<pumpkin_data::data_component_impl::MaxStackSizeImpl>()
                            .map_or(crate::inventory::BLOCK_STACK_SIZE, |component| {
                                component.size
                            });
                        *maximum <= limit
                    })),
            "Invalid creature loot"
        );
        let slot = &mut self.profiles[usize::from(profile.native.id)];
        assert!(slot.is_none(), "Duplicate creature profile");
        *slot = Some(profile);
        self.maximum_half_width = self.maximum_half_width.max(profile.body.half_width);
    }
    pub fn get(&self, id: SpeciesId) -> Option<&'static CreatureProfile> {
        self.profiles.get(usize::from(id.0)).copied().flatten()
    }
    pub(crate) fn maximum_half_width(&self) -> f32 {
        self.maximum_half_width
    }
}

#[derive(Component)]
#[component(immutable)]
#[require(Living)]
pub struct Creature(pub SpeciesId);

#[derive(Component, Default)]
#[require(VoxelNavigator)]
pub struct Wander;
#[derive(Component, Default)]
#[require(VoxelNavigator, ThreatMemory)]
pub struct Flee;
#[derive(Component, Default)]
#[require(VoxelNavigator)]
pub struct EscapeWater;

fn passive_quadruped(entity: &mut bevy::ecs::system::EntityCommands) {
    entity.insert((
        Wander,
        Flee,
        EscapeWater,
        LocomotionAnimation::default(),
        presentation::CreatureAudio::default(),
    ));
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CreatureSystems {
    Spawn,
    Decide,
    Navigate,
    Finish,
}

#[derive(Resource)]
pub(crate) struct CreatureRandom(u64);
impl Default for CreatureRandom {
    fn default() -> Self {
        Self(0x34d57f918e625cab)
    }
}
impl CreatureRandom {
    pub(crate) fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
    pub(crate) fn range(&mut self, count: u32) -> u32 {
        self.next() % count
    }
}

pub struct CreaturesPlugin;
pub struct CreatureBehaviorPlugin;
pub struct CreatureNavigationPlugin;
pub struct CreatureSpawningPlugin;
pub struct CreaturePresentationPlugin;
pub struct CreaturePersistencePlugin;

impl Plugin for CreaturesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CreatureCatalog>()
            .init_resource::<CreatureRandom>();
        {
            let mut catalog = app.world_mut().resource_mut::<CreatureCatalog>();
            catalog.register(&COW);
            catalog.register(&PIG);
        }
        app.configure_sets(
            FixedUpdate,
            (
                CreatureSystems::Spawn.before(CreatureSystems::Decide),
                CreatureSystems::Decide.before(CreatureSystems::Navigate),
                CreatureSystems::Navigate.before(crate::character::CharacterSystems),
                CreatureSystems::Finish.after(crate::survival::SurvivalSystems::Damage),
            )
                .in_set(crate::simulation::SimulationSystems::Actors),
        )
        .add_plugins((
            CreatureCombatPlugin,
            CreatureBehaviorPlugin,
            CreatureNavigationPlugin,
            CreatureSpawningPlugin,
            CreaturePresentationPlugin,
            CreaturePersistencePlugin,
        ));
    }
}
