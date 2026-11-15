use super::{DAY_LENGTH_SECS, TimeOfDay, night::SplitMix};
use crate::{game::GameState, settings::GraphicsSettings};
use bevy::{light::NotShadowReceiver, prelude::*};
use bevy_voxel::render::{
    environment::SkyPass,
    sky::{CloudFrame, CloudLayer, SkyAssets},
};
use std::sync::Arc;

pub(super) const CELL: f32 = 12.0;
pub(super) const PATTERN: i32 = 256;
const DRIFT_SPEED: f64 = 0.6;
const MIN_RADIUS: f32 = 192.0;
const MAX_RADIUS: f32 = 768.0;

pub(super) fn spawn_clouds(mut commands: Commands, mut assets: SkyAssets) {
    commands.spawn((
        CloudLayer::default(),
        assets.material(SkyPass::Clouds),
        Visibility::Hidden,
        NotShadowReceiver,
        DespawnOnExit(GameState::Game),
    ));
}

/// Horizontal distance where the clouds have faded out entirely, in blocks.
pub(crate) fn cloud_reach(settings: &GraphicsSettings) -> f32 {
    let reach = (crate::voxel::terrain_radius(settings) * 1.5).clamp(MIN_RADIUS, MAX_RADIUS);
    (reach / CELL).ceil() * CELL
}

pub(super) fn update_cloud_frame(
    time: Res<TimeOfDay>,
    settings: Res<GraphicsSettings>,
    mut frame: ResMut<CloudFrame>,
) {
    frame.drift = time.days() * DAY_LENGTH_SECS as f64 * DRIFT_SPEED;
    frame.reach = cloud_reach(&settings);
    frame.enabled = settings.clouds;
}

/// Tileable value-noise fBm, thresholded to about a third of the sky.
pub(super) fn generate_pattern() -> Arc<[bool]> {
    const OCTAVES: [(i32, f32); 3] = [(16, 0.55), (32, 0.3), (64, 0.15)];
    let mut random = SplitMix(0xc10d_5eed);
    let lattices: Vec<Vec<f32>> = OCTAVES
        .iter()
        .map(|&(size, _)| (0..size * size).map(|_| random.next()).collect())
        .collect();
    let sample = |lattice: &[f32], size: i32, x: f32, z: f32| {
        let (x0, z0) = (x.floor() as i32, z.floor() as i32);
        let (fx, fz) = (x - x0 as f32, z - z0 as f32);
        let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
        let at =
            |x: i32, z: i32| lattice[(x.rem_euclid(size) + z.rem_euclid(size) * size) as usize];
        let top = at(x0, z0).lerp(at(x0 + 1, z0), u);
        let bottom = at(x0, z0 + 1).lerp(at(x0 + 1, z0 + 1), u);
        top.lerp(bottom, v)
    };
    (0..PATTERN * PATTERN)
        .map(|index| {
            let (x, z) = ((index % PATTERN) as f32, (index / PATTERN) as f32);
            let density: f32 = OCTAVES
                .iter()
                .zip(&lattices)
                .map(|(&(size, weight), lattice)| {
                    let scale = size as f32 / PATTERN as f32;
                    weight * sample(lattice, size, x * scale, z * scale)
                })
                .sum();
            density > 0.56
        })
        .collect()
}
