use bevy_math::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct BlockBox {
    pub min: Vec3,
    pub max: Vec3,
}
/// One outward-facing quad in block-local coordinates.
#[derive(Clone, Copy, Debug)]
pub struct CuboidFace {
    pub axis: usize,
    pub positive: bool,
    pub positions: [Vec3; 4],
    pub normal: Vec3,
}

impl BlockBox {
    /// Six quads without allocation; callers select material and UVs per face.
    pub fn faces(self) -> impl Iterator<Item = CuboidFace> {
        (0..3).flat_map(move |axis| {
            [false, true].map(move |positive| {
                let mut origin = self.min;
                origin[axis] = if positive {
                    self.max[axis]
                } else {
                    self.min[axis]
                };
                let mut u = Vec3::ZERO;
                let mut v = Vec3::ZERO;
                u[(axis + 1) % 3] = self.size()[(axis + 1) % 3];
                v[(axis + 2) % 3] = self.size()[(axis + 2) % 3];
                let positions = if positive {
                    [origin, origin + u, origin + u + v, origin + v]
                } else {
                    [origin, origin + v, origin + u + v, origin + u]
                };
                let mut normal = Vec3::ZERO;
                normal[axis] = if positive { 1.0 } else { -1.0 };
                CuboidFace {
                    axis,
                    positive,
                    positions,
                    normal,
                }
            })
        })
    }
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn size(self) -> Vec3 {
        self.max - self.min
    }
    pub fn translated(self, delta: Vec3) -> Self {
        Self {
            min: self.min + delta,
            max: self.max + delta,
        }
    }
    pub fn ray(self, origin: Vec3, direction: Vec3) -> Option<(f32, Vec3)> {
        let mut enter = 0.0f32;
        let mut exit = f32::INFINITY;
        let axis =
            if direction.x.abs() >= direction.y.abs() && direction.x.abs() >= direction.z.abs() {
                0
            } else if direction.y.abs() >= direction.z.abs() {
                1
            } else {
                2
            };
        let mut normal = Vec3::ZERO;
        normal[axis] = -direction[axis].signum();
        for axis in 0..3 {
            if direction[axis].abs() < 1e-8 {
                if origin[axis] < self.min[axis] || origin[axis] > self.max[axis] {
                    return None;
                }
                continue;
            }
            let a = (self.min[axis] - origin[axis]) / direction[axis];
            let b = (self.max[axis] - origin[axis]) / direction[axis];
            let near = a.min(b);
            if near > enter {
                enter = near;
                normal = Vec3::ZERO;
                normal[axis] = -direction[axis].signum();
            }
            exit = exit.min(a.max(b));
            if enter > exit {
                return None;
            }
        }
        (exit >= 0.0).then_some((enter, normal))
    }
}
