//! Day and night: a shared sky gradient, a moving sun and moon that
//! light the terrain and hand its single shadow map to each other, stars, the moon's
//! phases and vanilla-style clouds.
//!
//! [`TimeOfDay`] advances with virtual time; [`SkyState`] derives every color and light
//! level from it once per frame and is the only source the rest of the game reads.

mod celestial;
mod clouds;
mod cycle;
mod night;

use crate::game::GameState;
use bevy::prelude::*;

pub(crate) use clouds::cloud_reach;
pub(crate) use cycle::SkyState;
pub use cycle::TimeOfDay;

/// Real seconds per in-game day: twice vanilla's, for a slower sun.
pub const DAY_LENGTH_SECS: f32 = 2400.0;
/// A new world starts in the early morning (days since creation).
pub(crate) const NEW_WORLD_TIME: f64 = 0.3;
/// How far the sun's and moon's paths lean toward +Z, in radians.
const ORBIT_TILT: f32 = 0.35;
/// Per axis, whether light can never come from that axis' negative side: the lights
/// stay above the horizon when they cast shadows, and on the +Z side of the sky.
/// Shadow meshes drop the faces turned toward those sides.
pub(crate) const LIGHT_NEVER_FROM: [bool; 3] = [false, true, true];

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum SkySystems {
    /// Advances [`TimeOfDay`] and derives [`SkyState`].
    Advance,
    /// Applies [`SkyState`] to lights, stars, the moon and clouds.
    Apply,
}

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        // Every sky pass draws with `hollow_render::environment::SkyMaterial`.
        app.init_resource::<TimeOfDay>()
            .init_resource::<SkyState>()
            .init_resource::<clouds::CloudPattern>()
            .configure_sets(
                Update,
                (SkySystems::Advance, SkySystems::Apply)
                    .chain()
                    .run_if(in_state(GameState::Game)),
            )
            .add_systems(
                OnEnter(GameState::Game),
                (
                    cycle::restore_time,
                    (
                        celestial::spawn_sky,
                        night::spawn_night_sky,
                        clouds::spawn_clouds,
                    ),
                )
                    .chain(),
            )
            .add_systems(
                FixedUpdate,
                cycle::advance_time.in_set(crate::simulation::SimulationSystems::Activation),
            )
            .add_systems(Update, cycle::update_sky_state.in_set(SkySystems::Advance))
            .add_systems(
                PostUpdate,
                celestial::update_background_bounds
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(bevy::camera::visibility::VisibilitySystems::CheckVisibility)
                    .run_if(in_state(GameState::Game)),
            )
            .add_systems(
                Update,
                (
                    celestial::update_lights,
                    celestial::update_sun_disk,
                    night::update_night_sky,
                    clouds::update_clouds,
                )
                    .in_set(SkySystems::Apply),
            );

        #[cfg(feature = "dev")]
        app.add_systems(
            Update,
            debug_time_controls
                .before(SkySystems::Advance)
                .run_if(in_state(GameState::Game)),
        );
    }
}

/// F7 fast-forwards a day per minute, F8 freezes the cycle, F9 skips a quarter day.
#[cfg(feature = "dev")]
fn debug_time_controls(keyboard: Res<ButtonInput<KeyCode>>, mut time: ResMut<TimeOfDay>) {
    const FAST: f32 = DAY_LENGTH_SECS / 60.0;
    if keyboard.just_pressed(KeyCode::F7) {
        time.speed = if time.speed == FAST { 1.0 } else { FAST };
    }
    if keyboard.just_pressed(KeyCode::F8) {
        time.speed = if time.speed == 0.0 { 1.0 } else { 0.0 };
    }
    if keyboard.just_pressed(KeyCode::F9) {
        time.advance(0.25);
    }
}
