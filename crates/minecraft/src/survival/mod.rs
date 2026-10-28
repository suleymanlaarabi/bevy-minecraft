//! Fixed-step living entities, player nutrition, consumption, and presentation.
use crate::inventory::catalog;
mod consume;
mod damage;
pub(crate) mod effects;
mod hud;
mod metabolism;
mod model;
mod placement;
mod presentation;

use bevy::prelude::*;
pub use consume::{ItemConsumed, ItemUsePlugin};
pub(crate) use consume::{cancel as cancel_use, choose_slot as choose_use_slot};
pub use damage::{DamageApplied, DamageRequest};
pub use model::UseInput;
pub use model::{
    ActiveEffect, ActiveEffects, ArmorStats, DamageState, Hazards, Health, Living, Nutrition,
    RespawnPoint, SurvivalRules, SurvivalSnapshot, UsingItem,
};
use model::{ItemUse, SurvivalRandom};
pub use presentation::PlayerSurvivalPlugin;

pub struct SurvivalPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurvivalSystems {
    Prepare,
    Environment,
    Consume,
    Metabolism,
    Damage,
}

pub(crate) fn mining_speed(effects: &ActiveEffects) -> f32 {
    use pumpkin_data::effect::StatusEffect as E;
    (1.0 + effects.level(E::HASTE.id) as f32 * 0.2)
        * match effects.level(E::MINING_FATIGUE.id) {
            0 => 1.0,
            1 => 0.3,
            2 => 0.09,
            3 => 0.0027,
            _ => 0.00081,
        }
}

impl Plugin for SurvivalPlugin {
    fn build(&self, app: &mut App) {
        use crate::character::CharacterSystems;
        app.init_resource::<SurvivalRules>()
            .add_plugins(ItemUsePlugin)
            .add_message::<DamageRequest>()
            .configure_sets(
                FixedUpdate,
                (
                    SurvivalSystems::Prepare.before(CharacterSystems),
                    SurvivalSystems::Environment.after(CharacterSystems),
                    SurvivalSystems::Consume.after(SurvivalSystems::Environment),
                    SurvivalSystems::Metabolism.after(SurvivalSystems::Consume),
                    SurvivalSystems::Damage.after(SurvivalSystems::Metabolism),
                )
                    .in_set(crate::simulation::SimulationSystems::Actors),
            )
            .add_systems(
                FixedUpdate,
                (
                    (
                        damage::advance_windows,
                        damage::refresh_equipment,
                        metabolism::locomotion,
                    )
                        .in_set(SurvivalSystems::Prepare),
                    metabolism::hazards.in_set(SurvivalSystems::Environment),
                    (effects::tick, metabolism::nutrition)
                        .chain()
                        .in_set(SurvivalSystems::Metabolism),
                    damage::resolve.in_set(SurvivalSystems::Damage),
                ),
            )
            .add_observer(damage::apply_knockback)
            .add_observer(metabolism::damage_response)
            .add_observer(exhaust);
    }
}

#[derive(EntityEvent)]
pub(crate) struct Exhaustion {
    pub entity: Entity,
    pub amount: f32,
}

fn exhaust(event: On<Exhaustion>, mut actors: Query<&mut Nutrition>) {
    if event.amount.is_finite()
        && event.amount > 0.0
        && let Ok(mut nutrition) = actors.get_mut(event.entity)
    {
        nutrition.exhaust(event.amount);
    }
}
