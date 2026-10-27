//! Pure support evaluation; bounded neighbor work belongs to its column.
use super::catalog::{SIDE_FACING, SIDES, SupportRule, profile, state_behavior};
use crate::voxel::{
    VoxelKind, VoxelState,
    edits::{CheckedEdit, EditBatch},
};
use bevy::prelude::*;

pub(crate) fn evaluate(
    position: IVec3,
    state: VoxelState,
    mut read: impl FnMut(IVec3) -> Option<VoxelState>,
) -> Result<Option<EditBatch>, IVec2> {
    use pumpkin_data::BlockDirection;
    let rule = profile(state.kind()).support;
    let (door, wall, vine) = (
        rule == SupportRule::Door,
        rule == SupportRule::WallMounted,
        rule == SupportRule::Vine,
    );
    if !door && !wall && !vine {
        return Ok(None);
    }
    let mut missing = None;
    let info = state_behavior(state);

    let mut sample = |p: IVec3| {
        if !(0..hollow_core::coordinates::CHUNK_HEIGHT).contains(&p.y) {
            return Some(VoxelKind::Air.default_state());
        }
        let value = read(p);
        if value.is_none() {
            missing = Some(crate::voxel::split_world_position(p).0);
        }
        value
    };
    let mut next = state;
    let mut paired_edit = None;
    let mut drop_item = false;
    match (door, wall, vine) {
        (true, _, _) => {
            let other_position = position + if info.upper { IVec3::NEG_Y } else { IVec3::Y };
            let other = sample(other_position);
            let paired = other.is_some_and(|other| {
                other.kind() == state.kind() && state_behavior(other).upper != info.upper
            });
            let supported = info.upper
                || sample(position - IVec3::Y)
                    .is_some_and(|s| s.native().is_side_solid(BlockDirection::Up));
            if !paired || !supported {
                next = state.removed();
                drop_item = !info.upper;
                if paired && let Some(other) = other {
                    paired_edit = Some(CheckedEdit {
                        position: other_position,
                        expected: other,
                        state: other.removed(),
                    });
                }
            }
        }
        (_, true, _) => {
            let direction = SIDES[info.facing as usize];
            let face = [
                BlockDirection::North,
                BlockDirection::East,
                BlockDirection::South,
                BlockDirection::West,
            ][info.facing as usize];
            if !sample(position - direction).is_some_and(|s| s.native().is_side_solid(face)) {
                next = state.removed();
                drop_item = true;
            }
        }
        (_, _, true) => {
            let mut surviving = info.attachments;
            for (i, delta) in SIDES.into_iter().chain([IVec3::Y]).enumerate() {
                if surviving & (1 << i) == 0 {
                    continue;
                }
                let face = [
                    BlockDirection::South,
                    BlockDirection::West,
                    BlockDirection::North,
                    BlockDirection::East,
                    BlockDirection::Down,
                ][i];
                let attached =
                    sample(position + delta).is_some_and(|s| s.native().is_side_solid(face));
                let inherited = i < 4
                    && sample(position + IVec3::Y).is_some_and(|s| {
                        s.kind() == state.kind() && state_behavior(s).attachments & (1 << i) != 0
                    });
                if !attached && !inherited {
                    surviving &= !(1 << i);
                    let side = if i == 4 {
                        pumpkin_data::block_properties::Facing::Up
                    } else {
                        SIDE_FACING[i]
                    };
                    next = next.with(&[crate::voxel::VoxelState::attachment(side, false)]);
                }
            }
            if surviving == 0 {
                next = state.removed();
                drop_item = true;
            }
        }
        _ => unreachable!("Support capability checked above"),
    }

    if let Some(missing) = missing {
        return Err(missing);
    }
    if next == state {
        return Ok(None);
    }
    let mut edits = Vec::with_capacity(1 + usize::from(paired_edit.is_some()));
    edits.extend(paired_edit);
    edits.push(CheckedEdit {
        position,
        expected: state,
        state: next,
    });
    Ok(Some(EditBatch {
        edits,
        consume: None,
        harvest: drop_item.then_some(super::loot::Harvest {
            position,
            state,
            offset: Vec3::ZERO,
            velocity: Vec3::Y * 1.2,
        }),
        tool: None,
    }))
}
