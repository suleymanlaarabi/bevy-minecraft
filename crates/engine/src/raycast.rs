//! Allocation-free voxel traversal with application-defined selection shapes.
use bevy_math::prelude::*;
use hollow_core::shape::BlockBox;

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub owner: IVec3,
    pub normal: Vec3,
    pub location: Vec3,
}

/// The owner radius includes models extending beyond their own voxel. The caller
/// supplies the actual shape policy; missing terrain stops traversal immediately.
pub fn raycast<T: Copy, I: Iterator<Item = BlockBox>>(
    origin: Vec3,
    direction: Dir3,
    reach: f32,
    owner_radius: i32,
    mut sample: impl FnMut(IVec3) -> Option<T>,
    mut shapes: impl FnMut(IVec3, T) -> Option<I>,
) -> Option<RayHit> {
    if !reach.is_finite() || reach < 0.0 || owner_radius < 0 || !origin.is_finite() {
        return None;
    }
    let direction = direction.as_vec3();
    let mut cell = origin.floor().as_ivec3();
    let step = direction.signum().as_ivec3();
    let delta = direction.recip().abs();
    let mut next = Vec3::splat(f32::INFINITY);
    for axis in 0..3 {
        if direction[axis] != 0.0 {
            let boundary = cell[axis] as f32 + if step[axis] > 0 { 1.0 } else { 0.0 };
            next[axis] = (boundary - origin[axis]) / direction[axis];
        }
    }
    let mut entered = 0.0;
    loop {
        sample(cell)?; // An unavailable chunk stops traversal.
        let end = next.min_element().min(reach);
        let mut best: Option<(f32, RayHit)> = None;
        // Shapes can overhang horizontally, rise above fences, or shift below plants.
        for dy in -owner_radius..=owner_radius {
            for dz in -owner_radius..=owner_radius {
                for dx in -owner_radius..=owner_radius {
                    let owner = cell + IVec3::new(dx, dy, dz);
                    let Some(state) = sample(owner) else {
                        continue;
                    };
                    let Some(shapes) = shapes(owner, state) else {
                        continue;
                    };
                    let mut inspect = |shape: BlockBox| {
                        let Some((distance, normal)) =
                            shape.translated(owner.as_vec3()).ray(origin, direction)
                        else {
                            return;
                        };
                        if distance + 1e-5 < entered || distance > end + 1e-5 || distance > reach {
                            return;
                        }
                        if best.as_ref().is_none_or(|(old, _)| distance < *old) {
                            best = Some((
                                distance,
                                RayHit {
                                    owner,
                                    normal,
                                    location: origin + direction * distance,
                                },
                            ));
                        }
                    };
                    for shape in shapes {
                        inspect(shape);
                    }
                }
            }
        }
        if let Some((_, target)) = best {
            return Some(target);
        }
        let axis = if next.x <= next.y && next.x <= next.z {
            0
        } else if next.y <= next.z {
            1
        } else {
            2
        };
        if next[axis] > reach {
            return None;
        }
        entered = next[axis];
        cell[axis] += step[axis];
        next[axis] += delta[axis];
    }
}
