//! Bounded placement queries shared by consumption and respawning.
use bevy::prelude::*;

use crate::{
    character::{Aabb, CharacterBody, Pose},
    voxel::{VoxelKind, VoxelWorld},
};

/// Shared contact classification for damage sampling and safe placement.
#[derive(Clone, Copy)]
pub(super) enum HazardKind {
    None,
    Fire,
    Cactus,
    Berry,
    HotFloor,
    Campfire,
    Lava,
}

pub(super) fn hazard(kind: VoxelKind) -> HazardKind {
    use pumpkin_data::BlockId;
    let Some(native) = kind.native_id() else {
        return HazardKind::None;
    };
    match native {
        BlockId::FIRE | BlockId::SOUL_FIRE => HazardKind::Fire,
        BlockId::CACTUS => HazardKind::Cactus,
        BlockId::SWEET_BERRY_BUSH => HazardKind::Berry,
        BlockId::MAGMA_BLOCK => HazardKind::HotFloor,
        BlockId::CAMPFIRE | BlockId::SOUL_CAMPFIRE => HazardKind::Campfire,
        BlockId::LAVA => HazardKind::Lava,
        _ => HazardKind::None,
    }
}

pub(super) fn on_support(
    world: &VoxelWorld,
    cell: IVec3,
    horizontal: Vec2,
    body: &CharacterBody,
    pose: Pose,
) -> Option<Vec3> {
    let block = world.state(cell)?;
    if block.is_liquid() || !matches!(hazard(block.kind()), HazardKind::None | HazardKind::Berry) {
        return None;
    }
    let top = block
        .collision_boxes(cell)
        .map(|shape| shape.max.y)
        .max_by(f32::total_cmp)?;
    // The small clearance also protects the collision check from f32 save rounding.
    let position = Vec3::new(
        horizontal.x,
        cell.y as f32 + top + body.origin_height + 0.001,
        horizontal.y,
    );
    let bounds = body.aabb(position.as_dvec3(), pose);
    let mut safe = true;
    world.visit_cells(
        bounds.min.floor().as_ivec3(),
        bounds.max.floor().as_ivec3(),
        |cell, block| {
            let Some(block) = block else {
                safe = false;
                return;
            };
            if block.is_liquid()
                || matches!(
                    hazard(block.kind()),
                    HazardKind::Lava | HazardKind::Fire | HazardKind::Berry
                )
            {
                safe = false;
            }
            let origin = cell.as_dvec3();
            if block.collision_boxes(cell).any(|shape| {
                bounds.intersects(&Aabb {
                    min: origin + shape.min.as_dvec3(),
                    max: origin + shape.max.as_dvec3(),
                })
            }) {
                safe = false;
            }
        },
    );
    safe.then_some(position)
}

pub(super) fn respawn(world: &VoxelWorld, position: Vec3, body: &CharacterBody) -> Option<Vec3> {
    let feet = position - Vec3::Y * body.origin_height;
    for radius in 0i32..=4 {
        for x in -radius..=radius {
            for z in -radius..=radius {
                if x.abs().max(z.abs()) != radius {
                    continue;
                }
                let horizontal = feet.xz() + Vec2::new(x as f32, z as f32);
                let center = Vec3::new(horizontal.x, feet.y, horizontal.y)
                    .floor()
                    .as_ivec3();
                for height in (0..=32)
                    .flat_map(|distance| [center.y - distance - 1, center.y + distance])
                    .filter(|&height| (0..=510).contains(&height) && height <= center.y + 8)
                {
                    if let Some(candidate) = on_support(
                        world,
                        center.with_y(height),
                        horizontal,
                        body,
                        Pose::Standing,
                    ) {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}
