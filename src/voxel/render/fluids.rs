//! Shared exposed-surface builder for uneven water and every lava state.
//! Classification and amounts are cached by native state, not parsed per cell.
use super::{
    meshing::{Padded, Sample},
    models::{self, ModelQuad},
};
use crate::voxel::state::BlockBox;
use bevy::prelude::*;
use pumpkin_data::block_properties::COLLISION_SHAPES;

const SIDES: [IVec3; 4] = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z];

#[inline]
pub(super) fn explicit_water(padded: &Padded, cell: IVec3) -> bool {
    let fluid = padded.get(cell).fluid();
    if fluid >> 4 != 1 {
        return false;
    }
    if fluid & 15 != 8 {
        return true;
    }
    // Greedy vertices cannot represent clipped faces. Only sections containing
    // partial occluders need these extra checks; open oceans keep the fast path.
    if padded.fluid_occluders
        && (padded.get(cell).fluid_occluder()
            || SIDES
                .into_iter()
                .chain([IVec3::Y, IVec3::NEG_Y])
                .any(|offset| padded.get(cell + offset).fluid_occluder()))
    {
        return true;
    }
    let above = padded.get(cell + IVec3::Y);
    if above.fluid() >> 4 == 1 || above.hides_water() {
        return false;
    }
    // All four cells sharing a corner must choose the same builder, including
    // diagonal shore contacts; otherwise a greedy ocean edge could leave a seam.
    SIDES
        .into_iter()
        .chain([
            IVec3::new(-1, 0, -1),
            IVec3::new(1, 0, -1),
            IVec3::new(-1, 0, 1),
            IVec3::new(1, 0, 1),
        ])
        .any(|offset| {
            let neighbor = padded.get(cell + offset);
            (neighbor.fluid() >> 4 == 1 && neighbor.fluid() & 15 != 8)
                || (neighbor.fluid() >> 4 != 1 && !neighbor.hides_water())
        })
}

fn corner_height(padded: &Padded, cell: IVec3, family: u8, x: i32, z: i32) -> f32 {
    let mut height = 0.0;
    let mut weight = 0.0;
    for dz in [z - 1, z] {
        for dx in [x - 1, x] {
            let position = cell + IVec3::new(dx, 0, dz);
            // A corner lookup stays inside the section's one-cell halo. No
            // diagonal-above sample is needed when this column is roofed.
            let sample = padded.get(position);
            if sample.fluid() >> 4 == family {
                if padded.get(position + IVec3::Y).fluid() >> 4 == family {
                    return 1.0;
                }
                let value = f32::from((sample.fluid() & 15).min(8)) / 9.0;
                let influence = if value >= 0.8 { 10.0 } else { 1.0 };
                height += value * influence;
                weight += influence;
            } else if !sample.hides_water() {
                weight += 1.0;
            }
        }
    }
    height / weight.max(1.0)
}

pub(super) fn visit(padded: &Padded, cell: IVec3, mut emit: impl FnMut(&ModelQuad)) {
    let sample = padded.get(cell);
    let family = sample.fluid() >> 4;
    let ceiling = padded.get(cell + IVec3::Y);
    let heights = if ceiling.fluid() >> 4 == family || ceiling.hides_water() {
        [1.0; 4]
    } else {
        [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(x, z)| corner_height(padded, cell, family, x, z))
    };
    let sloped = heights
        .iter()
        .any(|height| (height - heights[0]).abs() > 1e-5);
    for axis in 0..3 {
        for positive in [false, true] {
            let mut direction = IVec3::ZERO;
            direction[axis] = if positive { 1 } else { -1 };
            let neighbor = padded.get(cell + direction);
            if neighbor.fluid() >> 4 == family || neighbor.hides_water() {
                continue;
            }
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            let mut positions = [[0.0; 3]; 4];
            for (index, (a, b)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
                .into_iter()
                .enumerate()
            {
                let p = &mut positions[index];
                p[axis] = if positive { 1.0 } else { 0.0 };
                p[u] = a;
                p[v] = b;
                if p[1] == 1.0 {
                    let corner = match (p[0] as i32, p[2] as i32) {
                        (0, 0) => 0,
                        (1, 0) => 1,
                        (1, 1) => 2,
                        _ => 3,
                    };
                    p[1] = heights[corner];
                }
            }
            if !positive {
                positions.swap(1, 3);
            }
            let normal = (Vec3::from_array(positions[1]) - Vec3::from_array(positions[0]))
                .cross(Vec3::from_array(positions[3]) - Vec3::from_array(positions[0]))
                .normalize_or(direction.as_vec3());
            let uv = positions.map(|p| {
                if axis == 1 {
                    [p[0], p[2]]
                } else {
                    [p[if axis == 0 { 2 } else { 0 }], 1.0 - p[1]]
                }
            });
            let face = ModelQuad {
                positions,
                normal: normal.to_array(),
                uv,
                texture: models::fluid_texture(family, axis != 1 || sloped),
                tinted: false,
                cull: [0; 3],
                translucent: family == 1,
                effect: if family == 1 { 1 } else { 8 },
                light_emission: if family == 2 { 15 } else { 0 },
                shade_up: false,
            };
            visible_face(&face, sample, neighbor, direction.as_vec3(), &mut emit);
        }
    }
}

/// Native shapes are already cached per state. Non-occluding models must not
/// turn their selection bounds into solid fluid cuts.
fn occlusion_shapes(sample: Sample) -> &'static [u16] {
    if sample.fluid_occluder() {
        sample.state.native().outline_shapes
    } else {
        &[]
    }
}

fn visible_face(
    face: &ModelQuad,
    sample: Sample,
    neighbor: Sample,
    direction: Vec3,
    emit: &mut impl FnMut(&ModelQuad),
) {
    if !sample.fluid_occluder() && !neighbor.fluid_occluder() {
        emit(face);
        return;
    }
    let blockers = occlusion_shapes(sample)
        .iter()
        .map(|&id| BlockBox::from(COLLISION_SHAPES[id as usize]))
        .chain(occlusion_shapes(neighbor).iter().map(|&id| {
            BlockBox::from(COLLISION_SHAPES[id as usize]).translated(direction)
        }));
    let positions = face.positions.map(Vec3::from_array);
    let min = positions
        .into_iter()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let max = positions
        .into_iter()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    if !blockers.clone().any(|b| overlaps(min, max, b)) {
        emit(face);
        return;
    }
    // Clip the original triangles independently: a sloped water quad need not
    // be planar. Degenerate fourth vertices preserve the mesh's quad layout.
    let mut fragment = face.clone();
    let axis = if direction.y != 0.0 {
        1
    } else if direction.x != 0.0 {
        0
    } else {
        2
    };
    let mut emit_triangle = |triangle: [Vec3; 3]| {
        fragment.positions =
            [triangle[0], triangle[1], triangle[2], triangle[2]].map(|p| p.to_array());
        fragment.uv = fragment.positions.map(|p| {
            if axis == 1 {
                [p[0], p[2]]
            } else {
                [p[if axis == 0 { 2 } else { 0 }], 1.0 - p[1]]
            }
        });
        emit(&fragment);
    };
    for indices in [[0, 1, 2], [0, 2, 3]] {
        uncovered(
            indices.map(|i| positions[i]),
            blockers.clone(),
            &mut emit_triangle,
        );
    }
}

fn overlaps(min: Vec3, max: Vec3, b: BlockBox) -> bool {
    min.cmple(b.max).all() && max.cmpge(b.min).all()
}

/// A triangle clipped by six box planes has at most nine unique vertices, plus
/// a closing duplicate removed after each split. Scratch space stays on the stack.
struct Polygon {
    vertices: [Vec3; 10],
    len: usize,
}
impl Polygon {
    fn new() -> Self {
        Self {
            vertices: [Vec3::ZERO; 10],
            len: 0,
        }
    }
    fn push(&mut self, vertex: Vec3) {
        if self.len == 0 || self.vertices[self.len - 1].distance_squared(vertex) > 1e-12 {
            self.vertices[self.len] = vertex;
            self.len += 1;
        }
    }
    fn split(&self, axis: usize, plane: f32, sign: f32) -> (Self, Self) {
        let (mut inside, mut outside) = (Self::new(), Self::new());
        let distance = |p: Vec3| {
            let value = (p[axis] - plane) * sign;
            if value.abs() < 1e-6 {
                0.0
            } else {
                value
            }
        };
        let mut previous = self.vertices[self.len - 1];
        let mut previous_distance = distance(previous);
        for &current in &self.vertices[..self.len] {
            let current_distance = distance(current);
            if (previous_distance >= 0.0) != (current_distance >= 0.0) {
                let intersection = previous.lerp(
                    current,
                    previous_distance / (previous_distance - current_distance),
                );
                inside.push(intersection);
                outside.push(intersection);
            }
            if current_distance >= 0.0 {
                inside.push(current);
            } else {
                outside.push(current);
            }
            previous = current;
            previous_distance = current_distance;
        }
        for polygon in [&mut inside, &mut outside] {
            if polygon.len > 1
                && polygon.vertices[0].distance_squared(polygon.vertices[polygon.len - 1]) < 1e-12
            {
                polygon.len -= 1;
            }
        }
        (inside, outside)
    }
}

/// Subtract a union of solid boxes, retaining coplanar contact on the solid side.
/// Each surviving polygon is triangulated before processing the next box, which
/// bounds scratch space regardless of the number of boxes in a native shape.
fn uncovered(
    triangle: [Vec3; 3],
    mut blockers: impl Iterator<Item = BlockBox> + Clone,
    emit: &mut impl FnMut([Vec3; 3]),
) {
    if (triangle[1] - triangle[0])
        .cross(triangle[2] - triangle[0])
        .length_squared()
        < 1e-12
    {
        return;
    }
    let min = triangle
        .into_iter()
        .fold(Vec3::splat(f32::INFINITY), Vec3::min);
    let max = triangle
        .into_iter()
        .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
    while let Some(b) = blockers.next() {
        if !overlaps(min, max, b) {
            continue;
        }
        if min.cmpge(b.min).all() && max.cmple(b.max).all() {
            return;
        }
        let mut polygon = Polygon::new();
        for vertex in triangle {
            polygon.push(vertex);
        }
        for axis in 0..3 {
            for (plane, sign) in [(b.min[axis], 1.0), (b.max[axis], -1.0)] {
                let (inside, outside) = polygon.split(axis, plane, sign);
                for i in 1..outside.len.saturating_sub(1) {
                    uncovered(
                        [outside.vertices[0], outside.vertices[i], outside.vertices[i + 1]],
                        blockers.clone(),
                        emit,
                    );
                }
                if inside.len < 3 {
                    return;
                }
                polygon = inside;
            }
        }
        return;
    }
    emit(triangle);
}
