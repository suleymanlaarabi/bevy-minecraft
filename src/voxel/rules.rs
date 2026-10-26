//! Local placement and connection rules. No scheduled vanilla ticks or simulation.
use super::{VoxelKind, VoxelState, VoxelWorld};
use bevy::prelude::*;
use pumpkin_data::BlockDirection;
use pumpkin_data::{BlockId, Property as P, PropertyKey as K, block_properties as bp};
mod placement;
pub(crate) use placement::{Opening, SupportRule, item_form, profile};

const SIDES: [(bp::Facing, IVec3); 4] = [
    (bp::Facing::North, IVec3::NEG_Z),
    (bp::Facing::East, IVec3::X),
    (bp::Facing::South, IVec3::Z),
    (bp::Facing::West, IVec3::NEG_X),
];
fn facing(direction: Vec3) -> bp::Facing {
    if direction.x.abs() > direction.z.abs() {
        if direction.x > 0.0 {
            bp::Facing::East
        } else {
            bp::Facing::West
        }
    } else if direction.z > 0.0 {
        bp::Facing::South
    } else {
        bp::Facing::North
    }
}
fn direction(facing: bp::Facing) -> IVec3 {
    match facing {
        bp::Facing::North => IVec3::NEG_Z,
        bp::Facing::East => IVec3::X,
        bp::Facing::South => IVec3::Z,
        bp::Facing::West => IVec3::NEG_X,
        bp::Facing::Up => IVec3::Y,
        bp::Facing::Down => IVec3::NEG_Y,
    }
}
fn opposite(facing: bp::Facing) -> bp::Facing {
    match facing {
        bp::Facing::North => bp::Facing::South,
        bp::Facing::East => bp::Facing::West,
        bp::Facing::South => bp::Facing::North,
        bp::Facing::West => bp::Facing::East,
        bp::Facing::Up => bp::Facing::Down,
        bp::Facing::Down => bp::Facing::Up,
    }
}
fn wall_side(side: bp::Facing, connected: bool, tall: bool) -> P {
    macro_rules! height {
        ($variant:ident, $kind:ident) => {
            P::$variant(if !connected {
                bp::$kind::None
            } else if tall {
                bp::$kind::Tall
            } else {
                bp::$kind::Low
            })
        };
    }
    match side {
        bp::Facing::North => height!(NorthWall, NorthWall),
        bp::Facing::East => height!(EastWall, EastWall),
        bp::Facing::South => height!(SouthWall, SouthWall),
        bp::Facing::West => height!(WestWall, WestWall),
        _ => unreachable!("Horizontal wall side"),
    }
}
// Connection families, resolved once per block instead of by name on every edit.
const FENCE: u8 = 1;
const WALL: u8 = 1 << 1;
const PANE: u8 = 1 << 2;
const STAIR: u8 = 1 << 3;
const DRIPSTONE: u8 = 1 << 4;
const GATE: u8 = 1 << 5;
const CHEST: u8 = 1 << 6;
const CONNECTS: u8 = FENCE | WALL | PANE | STAIR | DRIPSTONE | CHEST;

fn family(kind: VoxelKind) -> u8 {
    placement::profile(kind).family
}
fn is_stair(kind: VoxelKind) -> bool {
    family(kind) == STAIR
}

pub(crate) fn has_connections(kind: VoxelKind) -> bool {
    family(kind) & CONNECTS != 0
}

pub(crate) fn connected(
    position: IVec3,
    state: VoxelState,
    mut sample: impl FnMut(IVec3) -> Option<VoxelState>,
) -> VoxelState {
    let own = family(state.kind());
    if own == CHEST {
        let face = state.facing();
        let forward = direction(face);
        let left = IVec3::new(forward.z, 0, -forward.x);
        let current = state
            .property(K::Type)
            .unwrap_or(P::ChestType(bp::ChestType::Single));
        // Existing pairs get first refusal. This prevents a third chest from
        // stealing a partner; missing chunks preserve generated pair states.
        for (offset, own_type, other_type) in [
            (
                -left,
                P::ChestType(bp::ChestType::Left),
                P::ChestType(bp::ChestType::Right),
            ),
            (
                left,
                P::ChestType(bp::ChestType::Right),
                P::ChestType(bp::ChestType::Left),
            ),
        ] {
            if current != P::ChestType(bp::ChestType::Single) && current != own_type {
                continue;
            }
            let Some(other) = sample(position + offset) else {
                if current == own_type {
                    return state;
                }
                continue;
            };
            if other.kind() == state.kind()
                && other.facing() == face
                && (other.property(K::Type) == Some(P::ChestType(bp::ChestType::Single))
                    || other.property(K::Type) == Some(other_type))
            {
                return state.with(&[own_type]);
            }
        }
        return state.with(&[P::ChestType(bp::ChestType::Single)]);
    }
    if own & (FENCE | PANE | WALL) != 0 {
        let wall = own == WALL;
        let mut connections = [false; 4];
        let mut known = [true; 4];
        for (index, &(key, delta)) in SIDES.iter().enumerate() {
            // Missing chunks preserve generated connections until that neighbor loads.
            let Some(neighbor) = sample(position + delta) else {
                connections[index] = state.properties().any(|property| {
                    property.key() == VoxelState::attachment(key, false).key()
                        && property != VoxelState::attachment(key, false)
                        && property != wall_side(key, false, false)
                });
                known[index] = false;
                continue;
            };
            let other = family(neighbor.kind());
            let gate = other == GATE && {
                let axis = direction(neighbor.facing());
                axis.dot(delta) == 0
            };
            let solid_face = neighbor.native().is_side_solid(match delta {
                IVec3::X => BlockDirection::West,
                IVec3::NEG_X => BlockDirection::East,
                IVec3::Z => BlockDirection::North,
                _ => BlockDirection::South,
            });
            connections[index] = solid_face || other == own || (own != PANE && gate);
        }
        let mut edits = [P::Up(false); 5];
        let mut count = 0;
        let above = if wall {
            sample(position + IVec3::Y)
        } else {
            None
        };
        let tall = above.is_some_and(VoxelState::full_collision);
        let post_above = tall
            || above.is_some_and(|s| {
                family(s.kind()) == WALL && s.property(K::Up) == Some(P::Up(true))
            });
        for (index, &(key, _)) in SIDES.iter().enumerate() {
            // A side facing a missing chunk keeps its connection, but a wall still
            // takes its height from the known cell above (tall or back to low).
            if !known[index] && !(wall && connections[index] && above.is_some()) {
                continue;
            }
            edits[count] = if wall {
                wall_side(key, connections[index], tall)
            } else {
                VoxelState::attachment(key, connections[index])
            };
            count += 1;
        }
        if wall {
            let straight = (connections[0] && connections[2] && !connections[1] && !connections[3])
                || (connections[1] && connections[3] && !connections[0] && !connections[2]);
            edits[count] = P::Up(!straight || post_above);
            count += 1;
        }
        return state.with(&edits[..count]);
    }
    if own == STAIR {
        let facing = state.facing();
        let forward = direction(facing);
        let half = state.property(K::Half);
        let left = IVec3::new(forward.z, 0, -forward.x);
        let same =
            |neighbor: VoxelState| is_stair(neighbor.kind()) && neighbor.property(K::Half) == half;
        let mut shape = bp::StairsShape::Straight;
        if let Some(front) = sample(position + forward).filter(|s| same(*s)) {
            let turn = direction(front.facing());
            if forward.dot(turn) == 0
                && sample(position - turn).is_none_or(|s| !same(s) || s.facing() != facing)
            {
                shape = if turn == left {
                    bp::StairsShape::OuterLeft
                } else {
                    bp::StairsShape::OuterRight
                };
            }
        }
        if shape == bp::StairsShape::Straight
            && let Some(back) = sample(position - forward).filter(|s| same(*s))
        {
            let turn = direction(back.facing());
            if forward.dot(turn) == 0
                && sample(position + turn).is_none_or(|s| !same(s) || s.facing() != facing)
            {
                shape = if turn == left {
                    bp::StairsShape::InnerLeft
                } else {
                    bp::StairsShape::InnerRight
                };
            }
        }
        return state.with(&[P::StairsShape(shape)]);
    }
    if own == DRIPSTONE {
        let up = state.property(K::VerticalDirection)
            == Some(P::VerticalDirection(bp::VerticalDirection::Up));
        let forward = if up { IVec3::Y } else { IVec3::NEG_Y };
        let front = sample(position + forward);
        let back = sample(position - forward);
        let same = |s: VoxelState| {
            s.kind() == state.kind()
                && s.property(K::VerticalDirection) == state.property(K::VerticalDirection)
        };
        let thickness = if front.is_some_and(|s| s.kind() == state.kind() && !same(s)) {
            bp::SpeleothemThickness::TipMerge
        } else if front.is_none_or(|s| !same(s)) {
            bp::SpeleothemThickness::Tip
        } else if front.is_some_and(|s| {
            matches!(
                s.property(K::Thickness),
                Some(P::Thickness(
                    bp::SpeleothemThickness::Tip | bp::SpeleothemThickness::TipMerge
                ))
            )
        }) {
            bp::SpeleothemThickness::Frustum
        } else if back.is_some_and(same) {
            bp::SpeleothemThickness::Middle
        } else {
            bp::SpeleothemThickness::Base
        };
        return state.with(&[P::Thickness(thickness)]);
    }
    state
}

pub(crate) fn paired_position(state: VoxelState, position: IVec3) -> Option<IVec3> {
    let pair = placement::profile(state.kind()).pair?;
    let values = pair.values();
    let delta = pair.delta(state);
    match state.property(pair.property()) {
        Some(value) if value == values[0] => Some(position + delta),
        Some(value) if value == values[1] => Some(position - delta),
        _ => None,
    }
}

pub(crate) fn paired_states(first: VoxelState, second: VoxelState) -> bool {
    first.kind() == second.kind()
        && placement::profile(first.kind()).pair.is_some_and(|pair| {
            first.property(pair.property()) != second.property(pair.property())
                && first.property(K::Facing) == second.property(K::Facing)
        })
}

pub(crate) fn placement(
    world: &VoxelWorld,
    kind: VoxelKind,
    destroy: IVec3,
    place: IVec3,
    hit: Vec3,
    normal: Vec3,
    look: Vec3,
) -> Option<Vec<(IVec3, VoxelState)>> {
    let clicked = world.state(destroy)?;
    let slab = placement::profile(kind).slab;
    if slab
        && clicked.kind() == kind
        && clicked.property(K::Type) != Some(P::SlabType(bp::SlabType::Double))
        && ((normal.y > 0.5
            && clicked.property(K::Type) == Some(P::SlabType(bp::SlabType::Bottom)))
            || (normal.y < -0.5
                && clicked.property(K::Type) == Some(P::SlabType(bp::SlabType::Top))))
    {
        return Some(vec![(
            destroy,
            clicked.with(&[P::SlabType(bp::SlabType::Double), P::Waterlogged(false)]),
        )]);
    }
    let position = if clicked.replaceable() && clicked.kind() != VoxelKind::Water {
        destroy
    } else {
        place
    };
    let old = world.state(position)?;
    if slab
        && old.kind() == kind
        && old.property(K::Type) != Some(P::SlabType(bp::SlabType::Double))
    {
        return Some(vec![(
            position,
            old.with(&[P::SlabType(bp::SlabType::Double), P::Waterlogged(false)]),
        )]);
    }
    if !old.replaceable() && !old.native().is_air() && old.kind() != VoxelKind::Water {
        return None;
    }
    let (mut state, mounted) = placement::mounted(kind, normal, look)?;
    let kind = state.kind();
    let id = kind.0;
    let face = facing(look);
    let wall = facing(normal);
    let upper = normal.y < -0.5 || (normal.y.abs() < 0.5 && hit.y.rem_euclid(1.0) > 0.5);
    if state.property(K::Facing).is_some() && !mounted {
        state = state.with(&[P::Facing(opposite(face))]);
    }
    let profile = placement::profile(kind);
    if matches!(profile.pair, Some(placement::Pair::Horizontal)) {
        state = state.with(&[P::Facing(face)]);
    }
    if mounted {
        let support = world.state(position - normal.round().as_ivec3())?;
        let side = match normal.round().as_ivec3() {
            IVec3::X => BlockDirection::East,
            IVec3::NEG_X => BlockDirection::West,
            IVec3::Z => BlockDirection::South,
            IVec3::NEG_Z => BlockDirection::North,
            IVec3::NEG_Y => BlockDirection::Down,
            _ => BlockDirection::Up,
        };
        let supported = if profile.hanging || profile.torch && normal.y > 0.5 {
            support.native().is_center_solid(side)
        } else if normal.y > 0.5 && profile.sign_or_banner {
            support.native().is_solid()
        } else {
            support.native().is_side_solid(side)
        };
        if !supported {
            return None;
        }
    }
    if profile.six_way {
        let orientation = if profile.facing_normal {
            normal
        } else if profile.facing_look {
            look
        } else {
            -look
        };
        let facing = if orientation.y.abs() > orientation.x.abs().max(orientation.z.abs()) {
            if orientation.y > 0.0 {
                bp::Facing::Up
            } else {
                bp::Facing::Down
            }
        } else {
            self::facing(orientation)
        };
        state = state.with(&[P::Facing(facing)]);
    }
    let axis = P::Axis(if normal.y.abs() > 0.5 {
        bp::Axis::Y
    } else if normal.x.abs() > 0.5 {
        bp::Axis::X
    } else {
        bp::Axis::Z
    });
    if state.supports(axis) {
        state = state.with(&[axis]);
    }
    if is_stair(kind) {
        state = state.with(&[
            P::Facing(face),
            P::Half(if upper {
                bp::Half::Top
            } else {
                bp::Half::Bottom
            }),
        ]);
    }
    if slab {
        state = state.with(&[P::SlabType(if upper {
            bp::SlabType::Top
        } else {
            bp::SlabType::Bottom
        })]);
    }
    if profile.opening == Opening::Trapdoor {
        state = state.with(&[
            P::Facing(if normal.y.abs() < 0.5 {
                wall
            } else {
                opposite(face)
            }),
            P::Half(if upper {
                bp::Half::Top
            } else {
                bp::Half::Bottom
            }),
        ]);
    }
    if matches!(profile.opening, Opening::Gate | Opening::Door) {
        state = state.with(&[P::Facing(face)]);
    }
    if id == BlockId::POINTED_DRIPSTONE {
        let vertical = if normal.y < 0.0 {
            bp::VerticalDirection::Down
        } else {
            bp::VerticalDirection::Up
        };
        let support = if vertical == bp::VerticalDirection::Down {
            position + IVec3::Y
        } else {
            position - IVec3::Y
        };
        if !world.state(support)?.native().is_center_solid(
            if vertical == bp::VerticalDirection::Down {
                BlockDirection::Down
            } else {
                BlockDirection::Up
            },
        ) && world.get(support) != Some(kind)
        {
            return None;
        }
        state = state.with(&[P::VerticalDirection(vertical)]);
    }
    if id == BlockId::LADDER {
        if normal.y.abs() > 0.5 {
            return None;
        }
        state = state.with(&[P::Facing(wall)]);
        let support = world.state(position - normal.round().as_ivec3())?;
        let side = match normal.round().as_ivec3() {
            IVec3::X => BlockDirection::East,
            IVec3::NEG_X => BlockDirection::West,
            IVec3::Z => BlockDirection::South,
            _ => BlockDirection::North,
        };
        if !support.native().is_side_solid(side) {
            return None;
        }
    }
    if id == BlockId::VINE || id == BlockId::GLOW_LICHEN {
        if id == BlockId::VINE && normal.y > 0.5 {
            return None;
        }
        if old.kind() == kind {
            state = old;
        }
        let attachment = if normal.y < -0.5 {
            bp::Facing::Up
        } else if normal.y > 0.5 {
            bp::Facing::Down
        } else {
            opposite(wall)
        };
        if !world
            .state(position - normal.round().as_ivec3())?
            .full_collision()
        {
            return None;
        }
        state = state.with(&[VoxelState::attachment(attachment, true)]);
    }
    if matches!(
        id,
        BlockId::SMALL_AMETHYST_BUD | BlockId::MEDIUM_AMETHYST_BUD | BlockId::LARGE_AMETHYST_BUD
    ) || id == BlockId::AMETHYST_CLUSTER
        || id == BlockId::END_ROD
    {
        state = state.with(&[P::Facing(if normal.y > 0.5 {
            bp::Facing::Up
        } else if normal.y < -0.5 {
            bp::Facing::Down
        } else {
            wall
        })]);
        if !world
            .state(position - normal.round().as_ivec3())?
            .full_collision()
        {
            return None;
        }
    }
    if profile.opening == Opening::Door {
        let side = Vec3::new(-look.z, 0.0, look.x).normalize_or_zero();
        let point = hit - position.as_vec3() - Vec3::splat(0.5);
        state = state.with(&[P::Hinge(if point.dot(side) > 0.0 {
            bp::DoorHinge::Right
        } else {
            bp::DoorHinge::Left
        })]);
    }
    let aquatic = matches!(
        id,
        BlockId::SEAGRASS | BlockId::TALL_SEAGRASS | BlockId::KELP | BlockId::KELP_PLANT
    );
    if aquatic && !old.has_water() {
        return None;
    }
    let below = world.state(position - IVec3::Y);
    let floor_support = !mounted
        && (matches!(
            id,
            BlockId::BAMBOO
                | BlockId::BAMBOO_SAPLING
                | BlockId::CACTUS
                | BlockId::SUGAR_CANE
                | BlockId::AZALEA
                | BlockId::FLOWERING_AZALEA
                | BlockId::SWEET_BERRY_BUSH
                | BlockId::BIG_DRIPLEAF
                | BlockId::BIG_DRIPLEAF_STEM
        ) || aquatic
            || profile.carpet
            || profile.opening == Opening::Door
            || id == BlockId::SNOW
            || placement::profile(kind).pair.is_some()
            || (state.native().collision_shapes.is_empty()
                && id != BlockId::VINE
                && id != BlockId::HANGING_ROOTS
                && id != BlockId::GLOW_LICHEN
                && !profile.torch
                && !matches!(
                    id,
                    BlockId::TORCHFLOWER | BlockId::POTTED_TORCHFLOWER | BlockId::TORCHFLOWER_CROP
                )));
    if floor_support
        && !below.is_some_and(|s| {
            s.native().is_side_solid(BlockDirection::Up)
                || ((aquatic
                    || id == BlockId::BAMBOO
                    || id == BlockId::CACTUS
                    || id == BlockId::SUGAR_CANE)
                    && s.kind() == kind)
        })
    {
        return None;
    }
    if id == BlockId::CACTUS {
        let ground = below?.kind().0;
        if !matches!(ground, BlockId::SAND | BlockId::RED_SAND | BlockId::CACTUS) {
            return None;
        }
        if SIDES.iter().any(|(_, delta)| {
            world
                .state(position + *delta)
                .is_some_and(|s| !s.native().collision_shapes.is_empty())
        }) {
            return None;
        }
    }
    if id == BlockId::HANGING_ROOTS && !world.state(position + IVec3::Y)?.full_collision() {
        return None;
    }
    if old.has_water() && state.property(K::Waterlogged).is_some() {
        state = state.with(&[P::Waterlogged(true)]);
    }
    state = connected(position, state, |pos| world.state(pos));
    let mut edits = vec![(position, state)];
    if let Some(upper) = paired_position(state, position) {
        let old_upper = world.state(upper)?;
        if !old_upper.replaceable() && !old_upper.native().is_air() {
            return None;
        }
        let pair = placement::profile(kind).pair?;
        let mut upper_state = state.with(&[pair.values()[1]]);
        if upper_state.property(K::Waterlogged).is_some() {
            upper_state = upper_state.with(&[P::Waterlogged(old_upper.has_water())]);
        }
        if matches!(pair, placement::Pair::Horizontal)
            && !world
                .state(upper - IVec3::Y)?
                .native()
                .is_side_solid(BlockDirection::Up)
        {
            return None;
        }
        edits.push((upper, upper_state));
    }
    Some(edits)
}
