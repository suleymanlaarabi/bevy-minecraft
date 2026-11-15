use crate::environment::SkyMaterial;
use bevy::{
    asset::RenderAssetUsages,
    mesh::PrimitiveTopology,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
use std::sync::Arc;

/// Immutable cloud geometry; pattern generation and art direction belong to the consumer.
#[derive(Resource)]
#[component(immutable)]
pub struct CloudGeometry {
    pub cell: f32,
    pub thickness: f32,
    pub altitude: f32,
    pub rebuild_margin: i32,
}
#[derive(Resource, Default)]
pub struct CloudFrame {
    pub drift: f64,
    pub reach: f32,
    pub enabled: bool,
}

/// Which cells hold a cloud, `PATTERN`² entries, generated once.
#[derive(Resource, Clone)]
pub struct CloudPattern {
    pub cells: Arc<[bool]>,
    pub side: i32,
}

impl CloudPattern {
    fn filled(&self, x: i32, z: i32) -> bool {
        self.cells[(x.rem_euclid(self.side) + z.rem_euclid(self.side) * self.side) as usize]
    }
}

/// Opaque, flat-shaded cloud boxes lit like terrain by the sun, moon and sky.
/// Their distance haze samples the same palette as the terrain.
///
/// The mesh covers the cells within `radius` of `center`, in pattern cells.
#[derive(Component, Default)]
pub struct CloudLayer {
    center: Option<IVec2>,
    radius: i32,
    building: Option<Task<(IVec2, i32, Mesh)>>,
}

/// Scrolls the layer with the wind and rebuilds its mesh in the background once the
/// camera has moved a few cells from its center.
#[allow(clippy::type_complexity)]
pub fn update_clouds(
    mut commands: Commands,
    frame: Res<CloudFrame>,
    geometry: Res<CloudGeometry>,
    pattern: Res<CloudPattern>,
    camera: Query<&GlobalTransform, With<crate::VoxelViewer>>,
    mut layers: Query<(
        Entity,
        &mut CloudLayer,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<SkyMaterial>,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let camera = camera.translation();
    let drift = frame.drift;
    let radius = (frame.reach / geometry.cell).ceil() as i32 + geometry.rebuild_margin;
    let wanted = IVec2::new(
        ((camera.x as f64 - drift) / geometry.cell as f64).floor() as i32,
        (camera.z / geometry.cell).floor() as i32,
    );

    for (entities, layers, mut transforms, mut visibility, materials_handles) in layers
        .contiguous_iter_mut()
        .expect("Dense cloud layer query")
    {
        for (index, layer) in layers.into_iter().enumerate() {
            let target = if frame.enabled {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            hollow_features::query::set_if_neq(&mut visibility, index, target);
            if !frame.enabled {
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
                commands
                    .entity(entities[index])
                    .insert(Mesh3d(meshes.add(mesh)));
                if let Some(mut material) = materials.get_mut(&materials_handles[index].0) {
                    let end = built_radius as f32 * geometry.cell
                        - geometry.rebuild_margin as f32 * geometry.cell;
                    material.settings.params = Vec4::new(end * 0.7, end, 0.0, 0.0);
                }
            }

            let stale = layer.center.is_none_or(|center| {
                (center - wanted).abs().max_element() >= geometry.rebuild_margin
                    || layer.radius != radius
            });
            if stale && layer.building.is_none() {
                let pattern = CloudPattern {
                    cells: pattern.cells.clone(),
                    side: pattern.side,
                };
                let geometry = CloudGeometry { ..*geometry };
                layer.building = Some(AsyncComputeTaskPool::get().spawn(async move {
                    (
                        wanted,
                        radius,
                        cloud_mesh(&pattern, &geometry, wanted, radius),
                    )
                }));
            }

            // The mesh is built around its own center: place that cell, plus the wind.
            if let Some(center) = layer.center {
                let translation = Vec3::new(
                    (center.x as f64 * geometry.cell as f64 + drift) as f32,
                    geometry.altitude,
                    center.y as f32 * geometry.cell,
                );
                hollow_features::query::set_if_neq(
                    &mut transforms,
                    index,
                    Transform::from_translation(translation),
                );
            }
        }
    }
}

/// Greedy rectangles on all six orientations preserve the exact cloud surface.
fn cloud_mesh(
    pattern: &CloudPattern,
    geometry: &CloudGeometry,
    center: IVec2,
    radius: i32,
) -> Mesh {
    let side = radius * 2 + 1;
    let dimensions = [side, 1, side];
    let scale = Vec3::new(geometry.cell, geometry.thickness, geometry.cell);
    let origin = Vec3::new(
        -radius as f32 * geometry.cell,
        0.0,
        -radius as f32 * geometry.cell,
    );
    let occupancy = (-radius..=radius)
        .flat_map(|z| {
            (-radius..=radius).map(move |x| {
                x * x + z * z <= radius * radius && pattern.filled(center.x + x, center.y + z)
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
    let indices = crate::model_mesh::compact_indices(indices, positions.len());
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(indices)
}
