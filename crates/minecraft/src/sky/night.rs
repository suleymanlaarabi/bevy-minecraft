use super::SkyState;
use crate::game::GameState;
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use hollow_render::environment::{SkyMaterial, SkyPass, SkySettings};

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
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    palette: Res<hollow_render::fog::SkyPalette>,
    sky: Res<SkyState>,
) {
    // Both are placed at infinity by their vertex shaders: their transforms only
    // turn the star field, and no bounding box can cull them.
    commands.spawn((
        Stars,
        Mesh3d(meshes.add(star_mesh())),
        MeshMaterial3d(materials.add(SkyMaterial {
            pass: SkyPass::Stars,
            settings: stars_settings(&sky),
            palette: palette.0.clone(),
        })),
        Transform::from_rotation(sky.sky_rotation),
        NoFrustumCulling,
        NotShadowCaster,
        DespawnOnExit(GameState::Game),
    ));
    commands.spawn((
        MoonDisk,
        Mesh3d(meshes.add(Rectangle::new(2.0, 2.0))),
        MeshMaterial3d(materials.add(SkyMaterial {
            pass: SkyPass::Moon,
            settings: moon_settings(&sky),
            palette: palette.0.clone(),
        })),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        DespawnOnExit(GameState::Game),
    ));
}

pub(super) fn update_night_sky(
    sky: Res<SkyState>,
    mut star_fields: Query<
        (
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<SkyMaterial>,
        ),
        (With<Stars>, Without<MoonDisk>),
    >,
    mut moons: Query<(&mut Visibility, &MeshMaterial3d<SkyMaterial>), With<MoonDisk>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    if !sky.is_changed() {
        return;
    }
    for (mut transform, mut visibility, material) in &mut star_fields {
        // Invisible in full daylight: skip drawing 2600 quads that add nothing.
        visibility.set_if_neq(if sky.night > 0.0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        transform.rotation = sky.sky_rotation;
        let settings = stars_settings(&sky);
        // Only re-upload while the brightness actually changes, at dusk and dawn.
        if materials
            .get(&material.0)
            .is_some_and(|stars| stars.settings != settings)
            && let Some(mut stars) = materials.get_mut(&material.0)
        {
            stars.settings = settings;
        }
    }
    let settings = moon_settings(&sky);
    // Fully transparent below the horizon: hidden, and its material left alone.
    let shown = settings.color.w > 0.0;
    for (mut visibility, material) in &mut moons {
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if shown && let Some(mut moon) = materials.get_mut(&material.0) {
            moon.settings = settings;
        }
    }
}

fn stars_settings(sky: &SkyState) -> SkySettings {
    SkySettings {
        params: Vec4::new(sky.night, 0.0, 0.0, 0.0),
        ..default()
    }
}

fn moon_settings(sky: &SkyState) -> SkySettings {
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
    let count = STAR_COUNT as usize;
    let mut positions = Vec::with_capacity(count * 4);
    let mut uvs = Vec::with_capacity(count * 4);
    let mut colors = Vec::with_capacity(count * 4);
    let mut indices = Vec::with_capacity(count * 6);
    for star in 0..STAR_COUNT {
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
        let (tangent, bitangent) = center.any_orthonormal_pair();
        for (u, v) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.push((center + (tangent * u + bitangent * v) * size).to_array());
            uvs.push([u, v]);
            colors.push(color);
        }
        let base = star * 4;
        // The camera sees the inside of the sphere: wind each face inward.
        indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
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
