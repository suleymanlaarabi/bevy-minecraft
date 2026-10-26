//! Immutable, registry-indexed capabilities. Nothing here is stored per terrain cell.
use crate::voxel::{VoxelKind, VoxelState};
use bevy::prelude::*;
use std::sync::LazyLock;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Opening {
    #[default]
    None,
    Door,
    Trapdoor,
    Gate,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Profile {
    pub opening: Opening,
    pub manual: bool,
    pub climbable: bool,
    pub chest: bool,
    pub support: SupportRule,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SupportRule {
    #[default]
    None,
    Door,
    WallMounted,
    Vine,
}

pub(crate) fn profile(kind: VoxelKind) -> &'static Profile {
    static PROFILES: LazyLock<Box<[Profile]>> = LazyLock::new(|| {
        VoxelKind::ALL
            .iter()
            .map(|kind| {
                let name = kind.name();
                let opening = if name.ends_with("_trapdoor") {
                    Opening::Trapdoor
                } else if name.ends_with("_door") {
                    Opening::Door
                } else if name.ends_with("_fence_gate") {
                    Opening::Gate
                } else {
                    Opening::None
                };
                Profile {
                    opening,
                    manual: opening != Opening::None
                        && !matches!(name, "iron_door" | "iron_trapdoor"),
                    climbable: matches!(name, "ladder" | "vine"),
                    chest: name == "chest",
                    support: if opening == Opening::Door {
                        SupportRule::Door
                    } else {
                        match name {
                            "ladder" => SupportRule::WallMounted,
                            "vine" => SupportRule::Vine,
                            _ => SupportRule::None,
                        }
                    },
                }
            })
            .collect()
    });
    &PROFILES[kind.index()]
}

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
pub(crate) const SIDE_NAMES: [&str; 4] = ["north", "east", "south", "west"];

pub(crate) fn state_behavior(state: VoxelState) -> &'static StateBehavior {
    static STATES: LazyLock<Box<[StateBehavior]>> = LazyLock::new(|| {
        (0..=u16::MAX)
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
                let properties = state.properties();
                let property = |key| {
                    properties
                        .iter()
                        .find(|property| property.key_str() == key)
                        .map(|property| property.value_str())
                };
                let open = property("open") == Some("true");
                let mut attachments = 0;
                if profile.support == SupportRule::Vine {
                    for (i, name) in SIDE_NAMES.iter().chain([&"up"]).enumerate() {
                        if property(name) == Some("true") {
                            attachments |= 1 << i;
                        }
                    }
                }
                StateBehavior {
                    climbable: profile.climbable,
                    upper: property("half") == Some("upper"),
                    facing: SIDE_NAMES
                        .iter()
                        .position(|name| Some(*name) == property("facing"))
                        .unwrap_or(0) as u8,
                    attachments,
                    open,
                    toggle: (profile.opening != Opening::None)
                        .then(|| state.with(&[("open", if open { "false" } else { "true" })])),
                    chest_half: match property("type") {
                        Some("left") => 1,
                        Some("right") => 2,
                        _ => 0,
                    },
                }
            })
            .collect()
    });
    &STATES[state.id() as usize]
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
const _: () = assert!(std::mem::size_of::<StateBehavior>() <= 12);
