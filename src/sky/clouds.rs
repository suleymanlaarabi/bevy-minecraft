#![allow(clippy::needless_arbitrary_self_type)]

use super::{DAY_LENGTH_SECS, TimeOfDay, night::SplitMix};
use crate::{game::GameState, player::PlayerCamera, settings::GraphicsSettings};
use bevy::{
    asset::RenderAssetUsages,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{Material, MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Face, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
use std::sync::Arc;

/// Vanilla's cloud layer: 12×12 block cells, 4 blocks thick, at Y 192.
const CELL: f32 = 12.0;
const THICKNESS: f32 = 4.0;
const ALTITUDE: f32 = 192.0;
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
/// Being opaque, they get the atmosphere's aerial perspective for free.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub(crate) struct CloudMaterial {
    /// Horizontal distances where the dithered fade starts and ends in XY.
    #[uniform(0)]
    fade: Vec4,
}

impl Material for CloudMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sky_clouds.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "shaders/sky_clouds.wesl".into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
        ])?];
        descriptor.primitive.cull_mode = Some(Face::Back);
        Ok(())
    }
}

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

pub(super) fn spawn_clouds(mut commands: Commands, mut materials: ResMut<Assets<CloudMaterial>>) {
    commands.spawn((
        CloudLayer::default(),
        MeshMaterial3d(materials.add(CloudMaterial { fade: Vec4::ZERO })),
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

/// Boxes for the filled cells within `radius` of `center`, relative to that cell's
/// corner. Top and bottom faces merge along X; sides only face empty cells.
fn cloud_mesh(pattern: &[bool], center: IVec2, radius: i32) -> Mesh {
    let filled = |x: i32, z: i32| {
        x * x + z * z <= radius * radius
            && CloudPattern::filled(pattern, center.x + x, center.y + z)
    };
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut quad = |corners: [Vec3; 4], normal: Vec3| {
        let base = positions.len() as u32;
        positions.extend(corners.map(|corner| corner.to_array()));
        normals.extend([normal.to_array(); 4]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    for z in -radius..=radius {
        let (z0, z1) = (z as f32 * CELL, (z + 1) as f32 * CELL);
        let mut x = -radius;
        while x <= radius {
            if !filled(x, z) {
                x += 1;
                continue;
            }
            let start = x;
            while x <= radius && filled(x, z) {
                x += 1;
            }
            let (x0, x1) = (start as f32 * CELL, x as f32 * CELL);
            quad(
                [
                    Vec3::new(x0, THICKNESS, z0),
                    Vec3::new(x0, THICKNESS, z1),
                    Vec3::new(x1, THICKNESS, z1),
                    Vec3::new(x1, THICKNESS, z0),
                ],
                Vec3::Y,
            );
            quad(
                [
                    Vec3::new(x0, 0.0, z0),
                    Vec3::new(x1, 0.0, z0),
                    Vec3::new(x1, 0.0, z1),
                    Vec3::new(x0, 0.0, z1),
                ],
                Vec3::NEG_Y,
            );
            quad(
                [
                    Vec3::new(x0, 0.0, z0),
                    Vec3::new(x0, 0.0, z1),
                    Vec3::new(x0, THICKNESS, z1),
                    Vec3::new(x0, THICKNESS, z0),
                ],
                Vec3::NEG_X,
            );
            quad(
                [
                    Vec3::new(x1, 0.0, z0),
                    Vec3::new(x1, THICKNESS, z0),
                    Vec3::new(x1, THICKNESS, z1),
                    Vec3::new(x1, 0.0, z1),
                ],
                Vec3::X,
            );
            for cell in start..x {
                let (c0, c1) = (cell as f32 * CELL, (cell + 1) as f32 * CELL);
                if !filled(cell, z - 1) {
                    quad(
                        [
                            Vec3::new(c0, 0.0, z0),
                            Vec3::new(c0, THICKNESS, z0),
                            Vec3::new(c1, THICKNESS, z0),
                            Vec3::new(c1, 0.0, z0),
                        ],
                        Vec3::NEG_Z,
                    );
                }
                if !filled(cell, z + 1) {
                    quad(
                        [
                            Vec3::new(c0, 0.0, z1),
                            Vec3::new(c1, 0.0, z1),
                            Vec3::new(c1, THICKNESS, z1),
                            Vec3::new(c0, THICKNESS, z1),
                        ],
                        Vec3::Z,
                    );
                }
            }
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
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
