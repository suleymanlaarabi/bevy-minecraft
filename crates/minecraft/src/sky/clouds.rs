#![allow(clippy::needless_arbitrary_self_type)]

use super::{DAY_LENGTH_SECS, TimeOfDay, night::SplitMix};
use crate::{game::GameState, player::PlayerCamera, settings::GraphicsSettings};
use bevy::{
    asset::RenderAssetUsages,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::PrimitiveTopology,
    prelude::*,
    render::render_resource::AsBindGroup,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
use std::sync::Arc;

/// Cloud layer: 12×12 block cells, 4 blocks thick, at Y 384.
const CELL: f32 = 12.0;
const THICKNESS: f32 = 4.0;
const ALTITUDE: f32 = 300.0;
/// Blocks per second along +X.
const DRIFT_SPEED: f64 = 0.6;
/// The pattern repeats every `PATTERN` cells on both axes.
const PATTERN: i32 = 256;
/// Clouds reach half again as far as the terrain, within these bounds (blocks).
const MIN_RADIUS: f32 = 192.0;
const MAX_RADIUS: f32 = 768.0;
/// Cells the camera may drift from the mesh center before it is rebuilt: the mesh
/// reaches this much past the fade, so its edge never shows.
const REBUILD_MARGIN: i32 = 2;

/// Opaque, flat-shaded cloud boxes lit like terrain by the sun, moon and sky.
/// Their distance haze samples the same palette as the terrain.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub(crate) struct CloudMaterial {
    /// Horizontal distances where the dithered fade starts and ends in XY.
    #[uniform(0)]
    fade: Vec4,
    #[storage(11, read_only, visibility(fragment))]
    palette: Handle<bevy::render::storage::ShaderBuffer>,
}

sky_material!(
    CloudMaterial,
    "embedded://hollow_render/shaders/sky_clouds.wesl",
    AlphaMode::Opaque,
    0.0,
    [ATTRIBUTE_POSITION => 0, ATTRIBUTE_NORMAL => 1],
    cull = Some(bevy::render::render_resource::Face::Back)
);

/// Which cells hold a cloud, `PATTERN`² entries, generated once.
#[derive(Resource, Clone)]
pub(super) struct CloudPattern(Arc<[bool]>);

impl Default for CloudPattern {
    fn default() -> Self {
        Self(generate_pattern())
    }
}

impl CloudPattern {
    fn filled(pattern: &[bool], x: i32, z: i32) -> bool {
        pattern[(x.rem_euclid(PATTERN) + z.rem_euclid(PATTERN) * PATTERN) as usize]
    }
}

/// The mesh covers the cells within `radius` of `center`, in pattern cells.
#[derive(Component, Default)]
pub(super) struct CloudLayer {
    center: Option<IVec2>,
    radius: i32,
    building: Option<Task<(IVec2, i32, Mesh)>>,
}

pub(super) fn spawn_clouds(
    mut commands: Commands,
    mut materials: ResMut<Assets<CloudMaterial>>,
    palette: Res<hollow_render::fog::SkyPalette>,
) {
    commands.spawn((
        CloudLayer::default(),
        MeshMaterial3d(materials.add(CloudMaterial {
            fade: Vec4::ZERO,
            palette: palette.0.clone(),
        })),
        Transform::default(),
        Visibility::Hidden,
        NotShadowCaster,
        NotShadowReceiver,
        DespawnOnExit(GameState::Game),
    ));
}

/// Horizontal distance where the clouds have faded out entirely, in blocks.
pub(crate) fn cloud_reach(settings: &GraphicsSettings) -> f32 {
    let reach = (crate::voxel::terrain_radius(settings) * 1.5).clamp(MIN_RADIUS, MAX_RADIUS);
    // The fade ends on the cell grid.
    (reach / CELL).ceil() * CELL
}

/// Scrolls the layer with the wind and rebuilds its mesh in the background once the
/// camera has moved a few cells from its center.
#[allow(clippy::type_complexity)]
pub(super) fn update_clouds(
    mut commands: Commands,
    time: Res<TimeOfDay>,
    settings: Res<GraphicsSettings>,
    pattern: Res<CloudPattern>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    mut layers: Query<(
        Entity,
        &mut CloudLayer,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<CloudMaterial>,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let camera = camera.translation();
    let drift = time.days() * DAY_LENGTH_SECS as f64 * DRIFT_SPEED;
    let radius = (cloud_reach(&settings) / CELL).ceil() as i32 + REBUILD_MARGIN;
    let wanted = IVec2::new(
        ((camera.x as f64 - drift) / CELL as f64).floor() as i32,
        (camera.z / CELL).floor() as i32,
    );

    for (entity, mut layer, mut transform, mut visibility, material) in &mut layers {
        visibility.set_if_neq(if settings.clouds {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !settings.clouds {
            layer.building = None;
            layer.center = None;
            continue;
        }

        if let Some(task) = &mut layer.building
            && let Some((center, built_radius, mesh)) = check_ready(task)
        {
            layer.building = None;
            layer.center = Some(center);
            layer.radius = built_radius;
            commands.entity(entity).insert(Mesh3d(meshes.add(mesh)));
            if let Some(mut material) = materials.get_mut(&material.0) {
                let end = built_radius as f32 * CELL - REBUILD_MARGIN as f32 * CELL;
                material.fade = Vec4::new(end * 0.7, end, 0.0, 0.0);
            }
        }

        let stale = layer.center.is_none_or(|center| {
            (center - wanted).abs().max_element() >= REBUILD_MARGIN || layer.radius != radius
        });
        if stale && layer.building.is_none() {
            let pattern = pattern.0.clone();
            layer.building = Some(
                AsyncComputeTaskPool::get()
                    .spawn(async move { (wanted, radius, cloud_mesh(&pattern, wanted, radius)) }),
            );
        }

        // The mesh is built around its own center: place that cell, plus the wind.
        if let Some(center) = layer.center {
            let translation = Vec3::new(
                (center.x as f64 * CELL as f64 + drift) as f32,
                ALTITUDE,
                center.y as f32 * CELL,
            );
            transform.set_if_neq(Transform::from_translation(translation));
        }
    }
}

/// Greedy rectangles on all six orientations preserve the exact cloud surface.
fn cloud_mesh(pattern: &[bool], center: IVec2, radius: i32) -> Mesh {
    let side = radius * 2 + 1;
    let dimensions = [side, 1, side];
    let scale = Vec3::new(CELL, THICKNESS, CELL);
    let origin = Vec3::new(-radius as f32 * CELL, 0.0, -radius as f32 * CELL);
    let occupancy = (-radius..=radius)
        .flat_map(|z| {
            (-radius..=radius).map(move |x| {
                x * x + z * z <= radius * radius
                    && CloudPattern::filled(pattern, center.x + x, center.y + z)
            })
        })
        .collect::<Vec<_>>();
    let filled = |cell: [i32; 3]| {
        cell.iter()
            .zip(dimensions)
            .all(|(&value, size)| (0..size).contains(&value))
            && occupancy[(cell[0] + cell[2] * side) as usize]
    };
    let mut positions = Vec::<[f32; 3]>::with_capacity(4096);
    let mut normals = Vec::<[f32; 3]>::with_capacity(4096);
    let mut indices = Vec::<u32>::with_capacity(6144);
    let mut mask = Vec::<i8>::new();
    for axis in 0..3 {
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let width = dimensions[u] as usize;
        let height = dimensions[v] as usize;
        mask.resize(width * height, 0);
        for plane in 0..=dimensions[axis] {
            for j in 0..height {
                for i in 0..width {
                    let mut cell = [0; 3];
                    cell[axis] = plane - 1;
                    cell[u] = i as i32;
                    cell[v] = j as i32;
                    let before = filled(cell);
                    cell[axis] += 1;
                    let after = filled(cell);
                    mask[i + j * width] = match (before, after) {
                        (true, false) => 1,
                        (false, true) => -1,
                        _ => 0,
                    };
                }
            }
            for j in 0..height {
                let mut i = 0;
                while i < width {
                    let sign = mask[i + j * width];
                    if sign == 0 {
                        i += 1;
                        continue;
                    }
                    let mut w = 1;
                    while i + w < width && mask[i + w + j * width] == sign {
                        w += 1;
                    }
                    let mut h = 1;
                    while j + h < height && (i..i + w).all(|x| mask[x + (j + h) * width] == sign) {
                        h += 1;
                    }
                    let mut corner = Vec3::ZERO;
                    corner[axis] = plane as f32;
                    corner[u] = i as f32;
                    corner[v] = j as f32;
                    let p = origin + corner * scale;
                    let mut du = Vec3::ZERO;
                    let mut dv = Vec3::ZERO;
                    du[u] = w as f32 * scale[u];
                    dv[v] = h as f32 * scale[v];
                    let corners = if sign > 0 {
                        [p, p + du, p + du + dv, p + dv]
                    } else {
                        [p, p + dv, p + du + dv, p + du]
                    };
                    let mut normal = Vec3::ZERO;
                    normal[axis] = sign as f32;
                    let base = positions.len() as u32;
                    positions.extend(corners.map(|corner| corner.to_array()));
                    normals.extend([normal.to_array(); 4]);
                    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                    for row in j..j + h {
                        mask[i + row * width..i + w + row * width].fill(0);
                    }
                    i += w;
                }
            }
        }
    }
    let indices = hollow_render::model_mesh::compact_indices(indices, positions.len());
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(indices)
}

/// Tileable value-noise fBm, thresholded to about a third of the sky.
fn generate_pattern() -> Arc<[bool]> {
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
