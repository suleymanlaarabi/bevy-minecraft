//! Local placement and connection rules. No scheduled vanilla ticks or simulation.
use super::{VoxelKind, VoxelState, VoxelWorld};
use bevy::prelude::*;
use pumpkin_data::BlockDirection;
use std::sync::LazyLock;
mod placement;
pub(crate) use placement::item_form;

const SIDES: [(&str, IVec3); 4] = [
    ("north", IVec3::NEG_Z),
    ("east", IVec3::X),
    ("south", IVec3::Z),
    ("west", IVec3::NEG_X),
];
fn facing(direction: Vec3) -> &'static str {
    if direction.x.abs() > direction.z.abs() {
        if direction.x > 0.0 { "east" } else { "west" }
    } else if direction.z > 0.0 {
        "south"
    } else {
        "north"
    }
}
fn direction(facing: &str) -> IVec3 {
    SIDES
        .iter()
        .find(|(key, _)| *key == facing)
        .map_or(IVec3::NEG_Z, |(_, d)| *d)
}
fn opposite(facing: &str) -> &'static str {
    self::facing(-direction(facing).as_vec3())
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
    static FAMILIES: LazyLock<Box<[u8]>> = LazyLock::new(|| {
        VoxelKind::ALL
            .iter()
            .map(|kind| {
                let name = kind.name();
                if name.ends_with("_fence") {
                    FENCE
                } else if name.ends_with("_wall") {
                    WALL
                } else if name.ends_with("_pane") || name == "iron_bars" {
                    PANE
                } else if name.ends_with("_stairs") {
                    STAIR
                } else if name == "pointed_dripstone" {
                    DRIPSTONE
                } else if name.ends_with("_fence_gate") {
                    GATE
                } else if kind.default_state().property("type") == Some("single")
                    && name.ends_with("chest")
                {
                    CHEST
                } else {
                    0
                }
            })
            .collect()
    });
    FAMILIES[kind.index()]
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
        let face = state.property("facing").unwrap_or("north");
        let forward = direction(face);
        let left = IVec3::new(forward.z, 0, -forward.x);
        let current = state.property("type").unwrap_or("single");
        // Existing pairs get first refusal. This prevents a third chest from
        // stealing a partner; missing chunks preserve generated pair states.
        for (offset, own_type, other_type) in [(-left, "left", "right"), (left, "right", "left")] {
            if current != "single" && current != own_type {
                continue;
            }
            let Some(other) = sample(position + offset) else {
                if current == own_type {
                    return state;
                }
                continue;
            };
            if other.kind() == state.kind()
                && other.property("facing") == Some(face)
                && (other.property("type") == Some("single")
                    || other.property("type") == Some(other_type))
            {
                return state.with(&[("type", own_type)]);
            }
        }
        return state.with(&[("type", "single")]);
    }
    if own & (FENCE | PANE | WALL) != 0 {
        let wall = own == WALL;
        let mut connections = [false; 4];
        let mut known = [true; 4];
        for (index, &(key, delta)) in SIDES.iter().enumerate() {
            // Missing chunks preserve generated connections until that neighbor loads.
            let Some(neighbor) = sample(position + delta) else {
                connections[index] = !matches!(state.property(key), Some("false" | "none") | None);
                known[index] = false;
                continue;
            };
            let other = family(neighbor.kind());
            let gate = other == GATE && {
                let axis = direction(neighbor.property("facing").unwrap_or("north"));
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
        let mut edits = [("", ""); 5];
        let mut count = 0;
        let above = if wall {
            sample(position + IVec3::Y)
        } else {
            None
        };
        let tall = above.is_some_and(VoxelState::full_collision);
        let post_above = tall
            || above.is_some_and(|s| family(s.kind()) == WALL && s.property("up") == Some("true"));
        for (index, &(key, _)) in SIDES.iter().enumerate() {
            // A side facing a missing chunk keeps its connection, but a wall still
            // takes its height from the known cell above (tall or back to low).
            if !known[index] && !(wall && connections[index] && above.is_some()) {
                continue;
            }
            let value = match (wall, connections[index]) {
                (true, true) if tall => "tall",
                (true, true) => "low",
                (true, false) => "none",
                (false, true) => "true",
                (false, false) => "false",
            };
            edits[count] = (key, value);
            count += 1;
        }
        if wall {
            let straight = (connections[0] && connections[2] && !connections[1] && !connections[3])
                || (connections[1] && connections[3] && !connections[0] && !connections[2]);
            edits[count] = (
                "up",
                if !straight || post_above {
                    "true"
                } else {
                    "false"
                },
            );
            count += 1;
        }
        return state.with(&edits[..count]);
    }
    if own == STAIR {
        let facing = state.property("facing").unwrap_or("north");
        let forward = direction(facing);
        let half = state.property("half");
        let left = IVec3::new(forward.z, 0, -forward.x);
        let same =
            |neighbor: VoxelState| is_stair(neighbor.kind()) && neighbor.property("half") == half;
        let mut shape = "straight";
        if let Some(front) = sample(position + forward).filter(|s| same(*s)) {
            let turn = direction(front.property("facing").unwrap_or("north"));
            if forward.dot(turn) == 0
                && sample(position - turn)
                    .is_none_or(|s| !same(s) || s.property("facing") != Some(facing))
            {
                shape = if turn == left {
                    "outer_left"
                } else {
                    "outer_right"
                };
            }
        }
        if shape == "straight"
            && let Some(back) = sample(position - forward).filter(|s| same(*s))
        {
            let turn = direction(back.property("facing").unwrap_or("north"));
            if forward.dot(turn) == 0
                && sample(position + turn)
                    .is_none_or(|s| !same(s) || s.property("facing") != Some(facing))
            {
                shape = if turn == left {
                    "inner_left"
                } else {
                    "inner_right"
                };
            }
        }
        return state.with(&[("shape", shape)]);
    }
    if own == DRIPSTONE {
        let up = state.property("vertical_direction") == Some("up");
        let forward = if up { IVec3::Y } else { IVec3::NEG_Y };
        let front = sample(position + forward);
        let back = sample(position - forward);
        let same = |s: VoxelState| {
            s.kind() == state.kind()
                && s.property("vertical_direction") == state.property("vertical_direction")
        };
        let thickness = if front.is_some_and(|s| s.kind() == state.kind() && !same(s)) {
            "tip_merge"
        } else if front.is_none_or(|s| !same(s)) {
            "tip"
        } else if front
            .is_some_and(|s| matches!(s.property("thickness"), Some("tip" | "tip_merge")))
        {
            "frustum"
        } else if back.is_some_and(same) {
            "middle"
        } else {
            "base"
        };
        return state.with(&[("thickness", thickness)]);
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
                && first.property("facing") == second.property("facing")
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
    let slab = kind.name().ends_with("_slab");
    if slab
        && clicked.kind() == kind
        && clicked.property("type") != Some("double")
        && ((normal.y > 0.5 && clicked.property("type") == Some("bottom"))
            || (normal.y < -0.5 && clicked.property("type") == Some("top")))
    {
        return Some(vec![(
            destroy,
            clicked.with(&[("type", "double"), ("waterlogged", "false")]),
        )]);
    }
    let position = if clicked.replaceable() && clicked.kind() != VoxelKind::Water {
        destroy
    } else {
        place
    };
    let old = world.state(position)?;
    if slab && old.kind() == kind && old.property("type") != Some("double") {
        return Some(vec![(
            position,
            old.with(&[("type", "double"), ("waterlogged", "false")]),
        )]);
    }
    if !old.replaceable() && !old.native().is_air() && old.kind() != VoxelKind::Water {
        return None;
    }
    let (mut state, mounted) = placement::mounted(kind, normal, look)?;
    let kind = state.kind();
    let name = kind.name();
    let face = facing(look);
    let wall = facing(normal);
    let upper = normal.y < -0.5 || (normal.y.abs() < 0.5 && hit.y.rem_euclid(1.0) > 0.5);
    if state.property("facing").is_some() && !mounted {
        state = state.with(&[("facing", opposite(face))]);
    }
    let profile = placement::profile(kind);
    if matches!(profile.pair, Some(placement::Pair::Horizontal)) {
        state = state.with(&[("facing", face)]);
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
        let supported = if profile.hanging || name.ends_with("torch") && normal.y > 0.5 {
            support.native().is_center_solid(side)
        } else if normal.y > 0.5 && (name.ends_with("_sign") || name.ends_with("_banner")) {
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
            if orientation.y > 0.0 { "up" } else { "down" }
        } else {
            self::facing(orientation)
        };
        state = state.with(&[("facing", facing)]);
    }
    if state.property("axis").is_some() {
        state = state.with(&[(
            "axis",
            if normal.y.abs() > 0.5 {
                "y"
            } else if normal.x.abs() > 0.5 {
                "x"
            } else {
                "z"
            },
        )]);
    }
    if is_stair(kind) {
        state = state.with(&[
            ("facing", face),
            ("half", if upper { "top" } else { "bottom" }),
        ]);
    }
    if slab {
        state = state.with(&[("type", if upper { "top" } else { "bottom" })]);
    }
    if name.ends_with("_trapdoor") {
        state = state.with(&[
            (
                "facing",
                if normal.y.abs() < 0.5 {
                    wall
                } else {
                    opposite(face)
                },
            ),
            ("half", if upper { "top" } else { "bottom" }),
        ]);
    }
    if name.ends_with("_fence_gate") || name.ends_with("_door") {
        state = state.with(&[("facing", face)]);
    }
    if name == "pointed_dripstone" {
        let vertical = if normal.y < 0.0 { "down" } else { "up" };
        let support = if vertical == "down" {
            position + IVec3::Y
        } else {
            position - IVec3::Y
        };
        if !world
            .state(support)?
            .native()
            .is_center_solid(if vertical == "down" {
                BlockDirection::Down
            } else {
                BlockDirection::Up
            })
            && world.get(support) != Some(kind)
        {
            return None;
        }
        state = state.with(&[("vertical_direction", vertical)]);
    }
    if name == "ladder" {
        if normal.y.abs() > 0.5 {
            return None;
        }
        state = state.with(&[("facing", wall)]);
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
    if name == "vine" || name == "glow_lichen" {
        if name == "vine" && normal.y > 0.5 {
            return None;
        }
        if old.kind() == kind {
            state = old;
        }
        let attachment = if normal.y < -0.5 {
            "up"
        } else if normal.y > 0.5 {
            "down"
        } else {
            opposite(wall)
        };
        if !world
            .state(position - normal.round().as_ivec3())?
            .full_collision()
        {
            return None;
        }
        state = state.with(&[(attachment, "true")]);
    }
    if name.contains("amethyst_bud") || name == "amethyst_cluster" || name == "end_rod" {
        state = state.with(&[(
            "facing",
            if normal.y > 0.5 {
                "up"
            } else if normal.y < -0.5 {
                "down"
            } else {
                wall
            },
        )]);
        if !world
            .state(position - normal.round().as_ivec3())?
            .full_collision()
        {
            return None;
        }
    }
    if name.ends_with("_door") {
        let side = Vec3::new(-look.z, 0.0, look.x).normalize_or_zero();
        let point = hit - position.as_vec3() - Vec3::splat(0.5);
        state = state.with(&[(
            "hinge",
            if point.dot(side) > 0.0 {
                "right"
            } else {
                "left"
            },
        )]);
    }
    let aquatic = matches!(name, "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant");
    if aquatic && !old.has_water() {
        return None;
    }
    let below = world.state(position - IVec3::Y);
    let floor_support = !mounted
        && (matches!(
            name,
            "bamboo"
                | "bamboo_sapling"
                | "cactus"
                | "sugar_cane"
                | "azalea"
                | "flowering_azalea"
                | "sweet_berry_bush"
                | "big_dripleaf"
                | "big_dripleaf_stem"
        ) || aquatic
            || name.ends_with("_carpet")
            || name.ends_with("_door")
            || name == "snow"
            || placement::profile(kind).pair.is_some()
            || (state.native().collision_shapes.is_empty()
                && name != "vine"
                && name != "hanging_roots"
                && name != "glow_lichen"
                && !name.contains("torch")));
    if floor_support
        && !below.is_some_and(|s| {
            s.native().is_side_solid(BlockDirection::Up)
                || ((aquatic || name == "bamboo" || name == "cactus" || name == "sugar_cane")
                    && s.kind() == kind)
        })
    {
        return None;
    }
    if name == "cactus" {
        let ground = below?.kind().name();
        if !matches!(ground, "sand" | "red_sand" | "cactus") {
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
    if name == "hanging_roots" && !world.state(position + IVec3::Y)?.full_collision() {
        return None;
    }
    if old.has_water() {
        state = state.with(&[("waterlogged", "true")]);
    }
    state = connected(position, state, |pos| world.state(pos));
    let mut edits = vec![(position, state)];
    if let Some(upper) = paired_position(state, position) {
        let old_upper = world.state(upper)?;
        if !old_upper.replaceable() && !old_upper.native().is_air() {
            return None;
        }
        let pair = placement::profile(kind).pair?;
        let upper_state = state.with(&[
            (pair.property(), pair.values()[1]),
            (
                "waterlogged",
                if old_upper.has_water() {
                    "true"
                } else {
                    "false"
                },
            ),
        ]);
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
