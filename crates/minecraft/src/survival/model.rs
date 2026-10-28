use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::inventory::ItemStack;

/// Capabilities shared by players and future living creatures.
#[derive(Component, Default)]
#[require(Health, ActiveEffects, DamageState, ArmorStats, Hazards)]
pub struct Living;

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub current: f32,
    pub maximum: f32,
    pub absorption: f32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            current: 20.0,
            maximum: 20.0,
            absorption: 0.0,
        }
    }
}

impl Health {
    pub fn alive(&self) -> bool {
        self.current > 0.0
    }

    pub fn heal(&mut self, amount: f32) {
        if self.alive() {
            self.current = (self.current + amount).min(self.maximum);
        }
    }
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Nutrition {
    pub food: u8,
    pub saturation: f32,
    pub exhaustion: f32,
    pub recovery_ticks: u16,
}

impl Default for Nutrition {
    fn default() -> Self {
        Self {
            food: 20,
            saturation: 5.0,
            exhaustion: 0.0,
            recovery_ticks: 0,
        }
    }
}

impl Nutrition {
    pub fn exhaust(&mut self, amount: f32) {
        self.exhaustion = (self.exhaustion + amount).min(40.0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveEffect {
    pub id: u8,
    pub amplifier: u8,
    pub ticks: u32,
    pub show_icon: bool,
}

impl From<&pumpkin_data::potion::Effect> for ActiveEffect {
    fn from(effect: &pumpkin_data::potion::Effect) -> Self {
        Self {
            id: effect.effect_type.id,
            amplifier: effect.amplifier,
            ticks: effect.duration.max(1) as u32,
            show_icon: effect.show_icon,
        }
    }
}

#[derive(Component, Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActiveEffects(pub Vec<ActiveEffect>);

impl ActiveEffects {
    pub fn level(&self, id: u8) -> u8 {
        self.0
            .iter()
            .find(|effect| effect.id == id)
            .map_or(0, |effect| effect.amplifier + 1)
    }
}

#[derive(Component, Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageState {
    pub invulnerable_ticks: u8,
    pub last_amount: f32,
    pub flash_ticks: u8,
    pub source: u8,
    pub direction: [f32; 3],
}

#[derive(Component, Default, Clone, Copy, PartialEq)]
pub struct ArmorStats {
    pub protection: f32,
    pub toughness: f32,
    pub knockback_resistance: f32,
}

#[derive(Component, Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hazards {
    pub fall_distance: f32,
    pub burning_ticks: u16,
    pub contact_ticks: u8,
    pub air_used: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemUse {
    pub slot: usize,
    pub stack: ItemStack,
    pub elapsed: u16,
    pub duration: u16,
    pub drinking: bool,
}

#[derive(Component, Default, Debug)]
#[require(UseInput, Health, ActiveEffects)]
pub struct UsingItem(pub Option<ItemUse>);

/// Held item-use intent. Any controller may set a slot; clearing it cancels use.
#[derive(Component, Default)]
pub struct UseInput(pub Option<usize>);

impl UsingItem {
    pub fn progress(&self) -> f32 {
        self.0
            .map_or(0.0, |action| action.elapsed as f32 / action.duration as f32)
    }
}

#[derive(Component, Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RespawnPoint(pub [f32; 3]);

#[derive(Bundle, Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurvivalSnapshot {
    pub health: Health,
    pub nutrition: Nutrition,
    pub effects: ActiveEffects,
    pub damage: DamageState,
    pub hazards: Hazards,
    pub respawn: RespawnPoint,
}

impl SurvivalSnapshot {
    pub fn validate(&self) -> bool {
        let n = &self.nutrition;
        validate_living(&self.health, &self.effects, &self.damage, &self.hazards)
            && n.food <= 20
            && n.saturation.is_finite()
            && (0.0..=n.food as f32).contains(&n.saturation)
            && n.exhaustion.is_finite()
            && (0.0..=40.0).contains(&n.exhaustion)
            && n.recovery_ticks <= 80
            && self
                .respawn
                .0
                .iter()
                .all(|value| value.is_finite() && value.abs() <= 30_000_000.0)
            && (-8.0..=512.0).contains(&self.respawn.0[1])
    }
}

pub(crate) fn validate_living(
    health: &Health,
    effects: &ActiveEffects,
    damage: &DamageState,
    hazards: &Hazards,
) -> bool {
    health.maximum.is_finite()
        && (1.0..=1024.0).contains(&health.maximum)
        && health.current.is_finite()
        && (0.0..=health.maximum).contains(&health.current)
        && health.absorption.is_finite()
        && (0.0..=256.0).contains(&health.absorption)
        && effects.0.len() <= 64
        && effects.0.iter().enumerate().all(|(index, effect)| {
            super::effects::supported(effect.id)
                && effect.amplifier <= 4
                && (1..=20 * 60 * 60 * 24).contains(&effect.ticks)
                && !effects.0[..index].iter().any(|other| other.id == effect.id)
        })
        && damage.last_amount.is_finite()
        && damage.last_amount >= 0.0
        && damage.direction.iter().all(|value| value.is_finite())
        && damage.invulnerable_ticks <= 20
        && damage.flash_ticks <= 10
        && hazards.fall_distance.is_finite()
        && hazards.fall_distance >= 0.0
        && hazards.air_used <= 320
}

#[derive(Resource)]
pub struct SurvivalRules {
    pub natural_regeneration: bool,
    pub starvation_floor: f32,
}

impl Default for SurvivalRules {
    fn default() -> Self {
        Self {
            natural_regeneration: true,
            starvation_floor: 1.0,
        }
    }
}

#[derive(Resource)]
pub(super) struct SurvivalRandom(u64);

impl Default for SurvivalRandom {
    fn default() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
                | 1,
        )
    }
}

impl SurvivalRandom {
    pub fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u32 << 24) as f32
    }
}
