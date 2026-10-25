use super::{DAY_LENGTH_SECS, ORBIT_TILT};
use bevy::{camera::Exposure, light::light_consts::lux, math::FloatPow, prelude::*};
use std::f32::consts::TAU;

/// Illuminance of the sun outside the atmosphere. The atmosphere reddens and dims it
/// on its way down, both for the sky and for terrain lighting.
pub(super) const SUN_LUX: f32 = lux::RAW_SUNLIGHT;
/// Far brighter than a real full moon (0.05 lux): night must stay playable.
pub(super) const MOON_LUX: f32 = 400.0;
/// Camera exposure at noon and in the middle of the night.
const DAY_EV100: f32 = 14.0;
const NIGHT_EV100: f32 = 9.0;
/// Sky irradiance on terrain open to the sky, as `AmbientLight` brightness.
const DAY_AMBIENT: f32 = 9_000.0;
const NIGHT_AMBIENT: f32 = 30.0;
/// The moon goes through its eight phases over eight days, like vanilla.
const MOON_PHASES: u32 = 8;
/// Lights only turn by whole steps: every turn re-rasterizes the shadow map along a
/// rotated texel grid, so each edge re-aliases and visibly crawls. A step comes every
/// ~3.3 s of a normal day and moves the shadow of a 10-block tree by about 9 cm.
/// The visible sun (`celestial::SunMaterial`) still moves smoothly.
const LIGHT_ANGLE_STEP: f32 = 0.5 * TAU / 360.0;

/// Time of day of the current world. `fraction` runs over one day: 0 is midnight,
/// 0.25 sunrise, 0.5 noon and 0.75 sunset.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TimeOfDay {
    pub day: u32,
    pub fraction: f32,
    /// Days per day length; 0 freezes the cycle.
    pub speed: f32,
}

impl Default for TimeOfDay {
    fn default() -> Self {
        Self::from_days(super::NEW_WORLD_TIME)
    }
}

impl TimeOfDay {
    pub(crate) fn from_days(days: f64) -> Self {
        let days = if days.is_finite() { days.max(0.0) } else { 0.0 };
        Self {
            day: days.floor().min(u32::MAX as f64) as u32,
            fraction: days.fract() as f32,
            speed: 1.0,
        }
    }

    /// Days elapsed since the world was created, as saved with the world.
    pub(crate) fn days(&self) -> f64 {
        self.day as f64 + self.fraction as f64
    }

    pub(crate) fn advance(&mut self, days: f32) {
        self.fraction += days;
        let whole = self.fraction.floor();
        if whole != 0.0 {
            self.fraction -= whole;
            self.day = self.day.saturating_add_signed(whole as i32);
        }
    }
}

/// Everything the renderer needs from the time of day, derived once per frame.
/// The single source of the sky's colors and light levels for every other system.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct SkyState {
    /// Unit vectors toward the sun and the moon, smooth for the visuals.
    pub sun_direction: Vec3,
    pub moon_direction: Vec3,
    /// The same directions, snapped to [`LIGHT_ANGLE_STEP`] for the lights.
    pub sun_light_direction: Vec3,
    pub moon_light_direction: Vec3,
    /// Orientation of the star field.
    pub sky_rotation: Quat,
    /// 0 in full daylight, 1 in the middle of the night.
    pub night: f32,
    pub ev100: f32,
    /// `AmbientLight` for terrain open to the sky.
    pub ambient_color: Color,
    pub ambient_brightness: f32,
    /// Fallback horizon color in output space while atmosphere LUTs are unavailable.
    pub horizon_color: Color,
    /// Lit fraction of the moon's disk, from 0 (new) to 1 (full), and whether it waxes.
    pub moon_phase: f32,
    pub moon_waxing: bool,
}

impl Default for SkyState {
    fn default() -> Self {
        Self::new(&TimeOfDay::default())
    }
}

impl SkyState {
    pub(crate) fn new(time: &TimeOfDay) -> Self {
        let angle = time.fraction * TAU;
        let snapped = (angle / LIGHT_ANGLE_STEP).round() * LIGHT_ANGLE_STEP;
        let sun_direction = orbit(angle);
        let moon_direction = opposite(sun_direction);
        let sun_light_direction = orbit(snapped);

        // Sine of the sun's elevation: negative at night.
        let elevation = sun_direction.y;
        let day = smoothstep(-0.2, 0.3, elevation);
        let ev100 = NIGHT_EV100.lerp(DAY_EV100, day);
        let ambient_brightness =
            NIGHT_AMBIENT * (DAY_AMBIENT / NIGHT_AMBIENT).powf(smoothstep(-0.25, 0.25, elevation));

        // Twilight warms the light within a few degrees of the horizon.
        let twilight = (1.0 - (elevation / 0.18).squared()).max(0.0);
        let ambient_color = Color::srgb(0.42, 0.52, 0.88)
            .mix(&Color::srgb(0.84, 0.92, 1.0), day)
            .mix(&Color::srgb(1.0, 0.72, 0.55), twilight * 0.45);
        let horizon_color = Color::srgb_u8(8, 12, 26)
            .mix(
                &Color::srgb_u8(186, 212, 245),
                smoothstep(-0.25, 0.25, elevation),
            )
            .mix(&Color::srgb_u8(236, 150, 104), twilight * 0.55);

        // Full on day 0, like vanilla; the phase advances at each midnight.
        let cycle = (time.day % MOON_PHASES) as f32 / MOON_PHASES as f32;
        Self {
            sun_direction,
            moon_direction,
            sun_light_direction,
            moon_light_direction: opposite(sun_light_direction),
            sky_rotation: Quat::from_rotation_x(-ORBIT_TILT) * Quat::from_rotation_z(angle),
            night: 1.0 - smoothstep(-0.25, 0.05, elevation),
            ev100,
            ambient_color,
            ambient_brightness,
            horizon_color,
            moon_phase: 0.5 + 0.5 * (cycle * TAU).cos(),
            moon_waxing: cycle >= 0.5,
        }
    }

    /// The camera exposure multiplier for [`Self::ev100`].
    pub(crate) fn exposure(&self) -> f32 {
        Exposure { ev100: self.ev100 }.exposure()
    }

    /// Whether the sun, not the moon, lights the terrain and casts its shadows.
    pub(crate) fn sun_is_up(&self) -> bool {
        self.sun_light_direction.y >= 0.0
    }
}

/// Direction toward the sun for a day angle: it rises in +X at a quarter turn and
/// crosses the sky tilted toward +Z, so neither light ever shines from -Z.
fn orbit(angle: f32) -> Vec3 {
    let (sin, cos) = angle.sin_cos();
    Vec3::new(sin, -cos, 0.0) * ORBIT_TILT.cos() + Vec3::Z * ORBIT_TILT.sin()
}

/// The moon faces the sun across the sky but on the same +Z side of it.
fn opposite(sun: Vec3) -> Vec3 {
    Vec3::new(-sun.x, -sun.y, sun.z)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Resumes the saved time of the world being entered, or starts a new world's morning.
pub(super) fn restore_time(
    session: Option<Res<crate::world::WorldSession>>,
    mut time_of_day: ResMut<TimeOfDay>,
    mut sky: ResMut<SkyState>,
) {
    let days = session
        .and_then(|session| session.resume.time_of_day)
        .unwrap_or(super::NEW_WORLD_TIME);
    *time_of_day = TimeOfDay {
        speed: time_of_day.speed,
        ..TimeOfDay::from_days(days)
    };
    *sky = SkyState::new(&time_of_day);
}

pub(super) fn advance_time(time: Res<Time>, mut time_of_day: ResMut<TimeOfDay>) {
    let days = time.delta_secs() * time_of_day.speed / DAY_LENGTH_SECS;
    if days != 0.0 {
        time_of_day.advance(days);
    }
}

pub(super) fn update_sky_state(time_of_day: Res<TimeOfDay>, mut sky: ResMut<SkyState>) {
    if time_of_day.is_changed() {
        sky.set_if_neq(SkyState::new(&time_of_day));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lights_never_shine_from_below_or_from_negative_z() {
        for step in 0..1000 {
            let sky = SkyState::new(&TimeOfDay::from_days(step as f64 / 1000.0));
            let light = if sky.sun_is_up() {
                sky.sun_light_direction
            } else {
                sky.moon_light_direction
            };
            assert!(light.y >= 0.0 && light.z > 0.0, "{light:?}");
        }
    }

    #[test]
    fn time_wraps_into_whole_days() {
        let mut time = TimeOfDay::from_days(2.75);
        time.advance(0.5);
        assert_eq!(time.day, 3);
        assert!((time.fraction - 0.25).abs() < 1e-6);
    }
}
