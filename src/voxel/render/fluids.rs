//! Shared exposed-surface builder for uneven water and every lava state.
//! Classification and amounts are cached by native state, not parsed per cell.
use super::{
    meshing::Padded,
    models::{self, ModelQuad},
};
use bevy::prelude::*;

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
            emit(&ModelQuad {
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
            });
        }
    }
}
