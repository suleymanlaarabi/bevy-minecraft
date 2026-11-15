use super::SkyState;
use crate::game::GameState;
use bevy::prelude::*;
use bevy_voxel::render::environment::{SkyPass, SkySettings};

const STAR_COUNT: u32 = 2600;
/// A slightly larger companion to the sun, with room to see its surface and phases.
const MOON_RADIUS: f32 = 0.035;

/// Point stars on a sphere at infinity, added over the sky gradient.
#[derive(Component)]
pub(super) struct Stars;

/// The moon's disk with its phases and a faint halo, added over the sky.
#[derive(Component)]
pub(super) struct MoonDisk;

pub(super) fn spawn_night_sky(
    mut commands: Commands,
    mut assets: bevy_voxel::render::sky::SkyAssets,
    sky: Res<SkyState>,
) {
    // Both are placed at infinity by their vertex shaders: their transforms only
    // turn the star field, and no bounding box can cull them.
    commands.spawn((
        Stars,
        assets.surface(star_mesh(), SkyPass::Stars),
        Transform::from_rotation(sky.sky_rotation),
        DespawnOnExit(GameState::Game),
    ));
    commands.spawn((
        MoonDisk,
        assets.surface(Rectangle::new(2.0, 2.0).into(), SkyPass::Moon),
        DespawnOnExit(GameState::Game),
    ));
}

/// Turns the star field and hides night bodies that add nothing. Star brightness and
/// the moon's disk follow the sky palette, written from [`moon_settings`].
pub(super) fn update_night_sky(
    sky: Res<SkyState>,
    mut star_fields: Query<(&mut Transform, &mut Visibility), (With<Stars>, Without<MoonDisk>)>,
    mut moons: Query<&mut Visibility, With<MoonDisk>>,
) {
    for (transforms, mut visibility) in star_fields
        .contiguous_iter_mut()
        .expect("Dense star field query")
    {
        // Invisible in full daylight: skip drawing 2600 quads that add nothing.
        let target = if sky.night > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        for (index, transform) in transforms.into_iter().enumerate() {
            bevy_voxel::features::query::set_if_neq(&mut visibility, index, target);
            transform.rotation = sky.sky_rotation;
        }
    }
    // Fully transparent below the horizon: hidden.
    let shown = moon_settings(&sky).color.w > 0.0;
    let target = if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in moons.contiguous_iter_mut().expect("Dense moon query") {
        for index in 0..visibility.len() {
            bevy_voxel::features::query::set_if_neq(&mut visibility, index, target);
        }
    }
}

/// Read through the sky palette ([`bevy_voxel::render::environment::SkyLighting`]).
pub(crate) fn moon_settings(sky: &SkyState) -> SkySettings {
    let direction = sky.moon_direction;
    // Dimmed and reddened by the long path through the air near the horizon,
    // and faint against the daylight sky.
    let altitude = direction.y.clamp(0.0, 1.0);
    let horizon = (altitude / 0.35).clamp(0.0, 1.0);
    let tint = Vec3::new(1.0, 0.62, 0.42).lerp(Vec3::new(0.96, 0.98, 1.0), horizon);
    let opacity = ((direction.y + 0.02) / 0.08).clamp(0.0, 1.0) * (0.2 + 0.8 * sky.night);
    SkySettings {
        direction: direction.extend(MOON_RADIUS),
        color: (tint * 1.6).extend(opacity),
        params: Vec4::new(
            sky.moon_phase,
            if sky.moon_waxing { 1.0 } else { -1.0 },
            0.0,
            0.0,
        ),
    }
}

/// Random stars as small camera-facing quads on the unit sphere, denser near the
/// faint end like the real sky. Seeded: every world sees the same constellations.
fn star_mesh() -> Mesh {
    let mut random = SplitMix(0x5eed_57a2);
    bevy_voxel::render::sky::star_mesh((0..STAR_COUNT).map(|_| {
        // Uniform on the sphere.
        let z = random.next() * 2.0 - 1.0;
        let angle = random.next() * std::f32::consts::TAU;
        let ring = (1.0 - z * z).sqrt();
        let center = Vec3::new(ring * angle.cos(), z, ring * angle.sin());
        let magnitude = random.next().powf(6.0);
        let size = 0.0022 + 0.0045 * magnitude;
        let brightness = 0.25 + 2.2 * magnitude;
        // Mostly white, a few blue and orange stars; A seeds the twinkle.
        let warmth = random.next() - 0.5;
        let color = [
            brightness * (1.0 + 0.25 * warmth),
            brightness,
            brightness * (1.0 - 0.3 * warmth),
            random.next(),
        ];
        bevy_voxel::render::sky::Star {
            center,
            size,
            color,
        }
    }))
}

/// A tiny deterministic generator for the star field and the cloud pattern.
pub(super) struct SplitMix(pub u64);

impl SplitMix {
    /// Uniform in `[0, 1)`.
    pub(super) fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}
