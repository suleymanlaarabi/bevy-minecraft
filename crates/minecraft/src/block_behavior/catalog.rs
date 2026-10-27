//! Immutable, registry-indexed capabilities. Nothing here is stored per terrain cell.
use crate::voxel::{VoxelKind, VoxelState};
use bevy::prelude::*;
use pumpkin_data::{Property as P, PropertyKey as K, block_properties as bp};
use std::sync::LazyLock;

pub(crate) use crate::voxel::rules::{Opening, SupportRule, profile};

#[derive(Clone, Copy, Default)]
pub(crate) struct StateBehavior {
    pub climbable: bool,
    pub upper: bool,
    pub facing: u8,
    pub attachments: u8,
    pub open: bool,
    pub toggle: Option<VoxelState>,
    pub chest_half: u8,
}

pub(crate) const SIDES: [IVec3; 4] = [IVec3::NEG_Z, IVec3::X, IVec3::Z, IVec3::NEG_X];
pub(crate) const SIDE_FACING: [bp::Facing; 4] = [
    bp::Facing::North,
    bp::Facing::East,
    bp::Facing::South,
    bp::Facing::West,
];

pub(crate) fn state_behavior(state: VoxelState) -> StateBehavior {
    static STATES: LazyLock<Box<[StateBehavior]>> = LazyLock::new(|| {
        (0..pumpkin_data::BlockStateId::COUNT)
            .map(|raw| {
                let Some(state) = VoxelState::from_id(raw) else {
                    return StateBehavior::default();
                };
                let profile = profile(state.kind());
                if profile.opening == Opening::None
                    && !profile.climbable
                    && !profile.chest
                    && profile.support == SupportRule::None
                {
                    return StateBehavior::default();
                }
                let open = state.property(K::Open) == Some(P::Open(true));
                let mut attachments = 0;
                if profile.support == SupportRule::Vine {
                    for (i, side) in SIDE_FACING.into_iter().chain([bp::Facing::Up]).enumerate() {
                        let property = crate::voxel::VoxelState::attachment(side, true);
                        if state.property(property.key()) == Some(property) {
                            attachments |= 1 << i;
                        }
                    }
                }
                StateBehavior {
                    climbable: profile.climbable,
                    upper: state.property(K::Half)
                        == Some(P::DoubleBlockHalf(bp::DoubleBlockHalf::Upper)),
                    facing: SIDE_FACING
                        .iter()
                        .position(|&facing| facing == state.facing())
                        .unwrap_or(0) as u8,
                    attachments,
                    open,
                    toggle: (profile.opening != Opening::None)
                        .then(|| state.with(&[P::Open(!open)])),
                    chest_half: match state.property(K::Type) {
                        Some(P::ChestType(bp::ChestType::Left)) => 1,
                        Some(P::ChestType(bp::ChestType::Right)) => 2,
                        _ => 0,
                    },
                }
            })
            .collect()
    });
    STATES.get(state.id() as usize).copied().unwrap_or_default()
}

pub(crate) fn chest_partner(position: IVec3, state: VoxelState) -> Option<IVec3> {
    let info = state_behavior(state);
    let forward = SIDES[info.facing as usize];
    let left = IVec3::new(forward.z, 0, -forward.x);
    match info.chest_half {
        1 => Some(position - left),
        2 => Some(position + left),
        _ => None,
    }
}

pub(crate) fn initialize() {
    // Force all allocations and property decoding out of gameplay hot paths.
    let _ = state_behavior(VoxelKind::Air.default_state());
}

// Keep the shared state table below 768 KiB, independent of loaded terrain size.
const _: () = assert!(std::mem::size_of::<StateBehavior>() <= 16);
