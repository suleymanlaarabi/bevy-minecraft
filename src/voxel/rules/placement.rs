//! Registry-derived placement capabilities, cached once per block family.
//! Wall aliases, vertical pairs and beds share descriptors instead of item lists.
use crate::voxel::{VoxelKind, VoxelState};
use bevy::prelude::*;
use std::sync::LazyLock;

#[derive(Clone, Copy)]
pub(super) enum Pair {
    Vertical,
    Horizontal,
}
impl Pair {
    pub(super) fn property(self) -> &'static str {
        match self {
            Self::Vertical => "half",
            Self::Horizontal => "part",
        }
    }
    pub(super) fn values(self) -> [&'static str; 2] {
        match self {
            Self::Vertical => ["lower", "upper"],
            Self::Horizontal => ["foot", "head"],
        }
    }
    pub(super) fn delta(self, state: VoxelState) -> IVec3 {
        match self {
            Self::Vertical => IVec3::Y,
            Self::Horizontal => super::direction(state.property("facing").unwrap_or("north")),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Profile {
    pub(super) item: VoxelKind,
    pub(super) wall: Option<VoxelKind>,
    pub(super) ceiling: bool,
    pub(super) pair: Option<Pair>,
    pub(super) rotation: bool,
    pub(super) face: bool,
    pub(super) attachment: bool,
    pub(super) hanging: bool,
    pub(super) six_way: bool,
    pub(super) facing_normal: bool,
    pub(super) facing_look: bool,
}

fn canonical(kind: VoxelKind) -> VoxelKind {
    let name = kind.name();
    let candidates = [
        name.replace("_wall_", "_"),
        name.strip_prefix("wall_").unwrap_or(name).to_owned(),
        name.strip_suffix("_plant").unwrap_or(name).to_owned(),
    ];
    candidates
        .iter()
        .filter(|candidate| candidate.as_str() != name)
        .filter_map(|name| VoxelKind::from_key(name))
        .find(|kind| kind.is_itemizable())
        .unwrap_or(kind)
}

pub(super) fn profile(kind: VoxelKind) -> &'static Profile {
    static PROFILES: LazyLock<Box<[Profile]>> = LazyLock::new(|| {
        VoxelKind::ALL
            .iter()
            .map(|&kind| {
                let item = canonical(kind);
                let name = item.name();
                let state = item.default_state();
                let wall = std::iter::once(format!("wall_{name}"))
                    .chain(
                        name.rsplit_once('_')
                            .map(|(base, suffix)| format!("{base}_wall_{suffix}")),
                    )
                    .chain(
                        name.strip_suffix("_hanging_sign")
                            .map(|base| format!("{base}_wall_hanging_sign")),
                    )
                    .find_map(|name| VoxelKind::from_key(&name));
                let pair = if state.property("half") == Some("lower") {
                    Some(Pair::Vertical)
                } else if state.property("part") == Some("foot") {
                    Some(Pair::Horizontal)
                } else {
                    None
                };
                Profile {
                    item,
                    wall,
                    pair,
                    ceiling: name.contains("hanging") && state.property("hanging").is_none(),
                    rotation: state.property("rotation").is_some(),
                    face: state.property("face").is_some(),
                    attachment: state.property("attachment").is_some(),
                    hanging: state.property("hanging").is_some(),
                    six_way: state.with(&[("facing", "up")]).property("facing") == Some("up"),
                    facing_normal: name.ends_with("shulker_box") || name == "lightning_rod",
                    facing_look: name == "observer",
                }
            })
            .collect()
    });
    &PROFILES[kind.index()]
}

pub(crate) fn item_form(kind: VoxelKind) -> Option<VoxelKind> {
    let item = profile(kind).item;
    item.is_itemizable().then_some(item)
}

pub(super) fn mounted(kind: VoxelKind, normal: Vec3, look: Vec3) -> Option<(VoxelState, bool)> {
    let profile = profile(kind);
    let mut state = if normal.y.abs() < 0.5 {
        profile.wall.unwrap_or(kind).default_state()
    } else {
        kind.default_state()
    };
    let mounted = profile.wall.is_some()
        || profile.face
        || profile.hanging
        || profile.ceiling
        || profile.attachment;
    if profile.ceiling && normal.y > 0.5
        || profile.ceiling && profile.wall.is_none() && normal.y.abs() < 0.5
        || profile.hanging && normal.y.abs() < 0.5
    {
        return None;
    }
    if profile.wall.is_some() && normal.y < -0.5 && !profile.ceiling {
        return None;
    }
    if profile.rotation && normal.y.abs() > 0.5 {
        let yaw = (-look.x).atan2(look.z);
        let rotation = ((yaw / std::f32::consts::TAU * 16.0 + 8.0).round() as i32).rem_euclid(16);
        state = state.with(&[("rotation", &rotation.to_string())]);
    }
    if profile.wall.is_some() && normal.y.abs() < 0.5 {
        state = state.with(&[("facing", super::facing(normal))]);
    }
    if profile.face || profile.attachment {
        state = state.with(&[(
            "facing",
            if normal.y.abs() < 0.5 {
                super::facing(normal)
            } else {
                super::opposite(super::facing(look))
            },
        )]);
        state = state.with(&[(
            if profile.face { "face" } else { "attachment" },
            if normal.y > 0.5 {
                "floor"
            } else if normal.y < -0.5 {
                "ceiling"
            } else {
                if profile.face { "wall" } else { "single_wall" }
            },
        )]);
    }
    if profile.hanging {
        state = state.with(&[("hanging", if normal.y < -0.5 { "true" } else { "false" })]);
    }
    Some((state, mounted))
}
