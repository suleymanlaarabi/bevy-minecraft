//! Registry-derived placement capabilities, cached once per block family.
//! Wall aliases, vertical pairs and beds share descriptors instead of item lists.
use crate::voxel::{VoxelKind, VoxelState};
use bevy::prelude::*;
use pumpkin_data::{
    BlockId, Property as P, PropertyKey as K, block_properties as bp, tag::Block as Tag,
};
use std::sync::LazyLock;

#[derive(Clone, Copy)]
pub(crate) enum Pair {
    Vertical,
    Horizontal,
}
impl Pair {
    pub(crate) fn property(self) -> K {
        match self {
            Self::Vertical => K::Half,
            Self::Horizontal => K::Part,
        }
    }
    pub(crate) fn values(self) -> [P; 2] {
        match self {
            Self::Vertical => [
                P::DoubleBlockHalf(bp::DoubleBlockHalf::Lower),
                P::DoubleBlockHalf(bp::DoubleBlockHalf::Upper),
            ],
            Self::Horizontal => [P::Part(bp::BedPart::Foot), P::Part(bp::BedPart::Head)],
        }
    }
    pub(crate) fn delta(self, state: VoxelState) -> IVec3 {
        match self {
            Self::Vertical => IVec3::Y,
            Self::Horizontal => super::direction(state.facing()),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Opening {
    #[default]
    None,
    Door,
    Trapdoor,
    Gate,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum SupportRule {
    #[default]
    None,
    Door,
    WallMounted,
    Vine,
}

#[derive(Clone, Copy)]
pub(crate) struct Profile {
    pub(crate) item: VoxelKind,
    pub(crate) wall: Option<VoxelKind>,
    pub(crate) ceiling: bool,
    pub(crate) pair: Option<Pair>,
    pub(crate) rotation: bool,
    pub(crate) face: bool,
    pub(crate) attachment: bool,
    pub(crate) hanging: bool,
    pub(crate) six_way: bool,
    pub(crate) facing_normal: bool,
    pub(crate) facing_look: bool,
    pub family: u8,
    pub opening: Opening,
    pub manual: bool,
    pub climbable: bool,
    pub chest: bool,
    pub support: SupportRule,
    pub slab: bool,
    pub torch: bool,
    pub sign_or_banner: bool,
    pub carpet: bool,
}

pub(crate) fn profile(kind: VoxelKind) -> &'static Profile {
    static PROFILES: LazyLock<Box<[Profile]>> = LazyLock::new(|| {
        VoxelKind::ALL
            .iter()
            .map(|&kind| {
                let item = VoxelKind(hollow_worldgen::block_relations::canonical(kind.0));
                let state = item.default_state();
                let wall = hollow_worldgen::block_relations::wall(kind.0).map(VoxelKind);
                let pair = if state.property(K::Half)
                    == Some(P::DoubleBlockHalf(bp::DoubleBlockHalf::Lower))
                {
                    Some(Pair::Vertical)
                } else if state.property(K::Part) == Some(P::Part(bp::BedPart::Foot)) {
                    Some(Pair::Horizontal)
                } else {
                    None
                };
                let opening = if kind.tagged(Tag::MINECRAFT_TRAPDOORS) {
                    Opening::Trapdoor
                } else if kind.tagged(Tag::MINECRAFT_DOORS) {
                    Opening::Door
                } else if kind.tagged(Tag::MINECRAFT_FENCE_GATES) {
                    Opening::Gate
                } else {
                    Opening::None
                };
                let family = if kind.tagged(Tag::MINECRAFT_FENCES) {
                    super::FENCE
                } else if kind.tagged(Tag::MINECRAFT_WALLS) {
                    super::WALL
                } else if kind.tagged(Tag::C_GLASS_PANES) || kind.0 == BlockId::IRON_BARS {
                    super::PANE
                } else if kind.tagged(Tag::MINECRAFT_STAIRS) {
                    super::STAIR
                } else if kind.0 == BlockId::POINTED_DRIPSTONE {
                    super::DRIPSTONE
                } else if opening == Opening::Gate {
                    super::GATE
                } else if kind.default_state().property(K::Type)
                    == Some(P::ChestType(bp::ChestType::Single))
                {
                    super::CHEST
                } else {
                    0
                };
                Profile {
                    item,
                    wall,
                    pair,
                    ceiling: (item.tagged(Tag::MINECRAFT_ALL_HANGING_SIGNS)
                        || item.0 == BlockId::HANGING_ROOTS)
                        && state.property(K::Hanging).is_none(),
                    rotation: state.property(K::Rotation).is_some(),
                    face: state.property(K::Face).is_some(),
                    attachment: state.property(K::Attachment).is_some(),
                    hanging: state.property(K::Hanging).is_some(),
                    six_way: state.supports(P::Facing(bp::Facing::Up)),
                    facing_normal: item.tagged(Tag::MINECRAFT_SHULKER_BOXES)
                        || item.0 == BlockId::LIGHTNING_ROD,
                    facing_look: item.0 == BlockId::OBSERVER,
                    family,
                    opening,
                    manual: opening != Opening::None
                        && !matches!(kind.0, BlockId::IRON_DOOR | BlockId::IRON_TRAPDOOR),
                    climbable: matches!(kind.0, BlockId::LADDER | BlockId::VINE),
                    chest: kind.0 == BlockId::CHEST,
                    support: if opening == Opening::Door {
                        SupportRule::Door
                    } else if kind.0 == BlockId::LADDER {
                        SupportRule::WallMounted
                    } else if kind.0 == BlockId::VINE {
                        SupportRule::Vine
                    } else {
                        SupportRule::None
                    },
                    slab: kind.tagged(Tag::MINECRAFT_SLABS),
                    torch: matches!(
                        kind.0,
                        BlockId::TORCH
                            | BlockId::WALL_TORCH
                            | BlockId::SOUL_TORCH
                            | BlockId::SOUL_WALL_TORCH
                            | BlockId::REDSTONE_TORCH
                            | BlockId::REDSTONE_WALL_TORCH
                            | BlockId::COPPER_TORCH
                            | BlockId::COPPER_WALL_TORCH
                    ),
                    sign_or_banner: kind.tagged(Tag::MINECRAFT_SIGNS)
                        || kind.tagged(Tag::MINECRAFT_ALL_HANGING_SIGNS)
                        || kind.tagged(Tag::MINECRAFT_BANNERS),
                    carpet: kind.tagged(Tag::MINECRAFT_WOOL_CARPETS)
                        || matches!(kind.0, BlockId::MOSS_CARPET | BlockId::PALE_MOSS_CARPET),
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

pub(crate) fn mounted(kind: VoxelKind, normal: Vec3, look: Vec3) -> Option<(VoxelState, bool)> {
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
        state = state.with(&[P::Rotation(rotation as u8)]);
    }
    if profile.wall.is_some() && normal.y.abs() < 0.5 {
        state = state.with(&[P::Facing(super::facing(normal))]);
    }
    if profile.face || profile.attachment {
        state = state.with(&[P::Facing(if normal.y.abs() < 0.5 {
            super::facing(normal)
        } else {
            super::opposite(super::facing(look))
        })]);
        let property = if profile.face {
            P::Face(if normal.y > 0.5 {
                bp::AttachFace::Floor
            } else if normal.y < -0.5 {
                bp::AttachFace::Ceiling
            } else {
                bp::AttachFace::Wall
            })
        } else {
            P::Attachment(if normal.y > 0.5 {
                bp::BellAttachment::Floor
            } else if normal.y < -0.5 {
                bp::BellAttachment::Ceiling
            } else {
                bp::BellAttachment::SingleWall
            })
        };
        state = state.with(&[property]);
    }
    if profile.hanging {
        state = state.with(&[P::Hanging(if normal.y < -0.5 { true } else { false })]);
    }
    Some((state, mounted))
}
