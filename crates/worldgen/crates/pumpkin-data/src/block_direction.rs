use crate::block_properties::{Axis, Facing, HorizontalAxis, HorizontalFacing};
use pumpkin_util::{
    math::vector3::{Axis as MathAxis, Vector3},
    random::{RandomGenerator, RandomImpl},
};
use serde::Deserialize;

#[repr(u8)]
#[derive(PartialEq, Clone, Copy, Debug, Hash, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockDirection {
    Down = 0,
    Up,
    North,
    South,
    West,
    East,
}

impl From<MathAxis> for Axis {
    fn from(a: MathAxis) -> Self {
        match a {
            MathAxis::X => Self::X,
            MathAxis::Y => Self::Y,
            MathAxis::Z => Self::Z,
        }
    }
}
impl From<Axis> for MathAxis {
    fn from(a: Axis) -> Self {
        match a {
            Axis::X => Self::X,
            Axis::Y => Self::Y,
            Axis::Z => Self::Z,
        }
    }
}

impl From<pumpkin_util::BlockDirection> for BlockDirection {
    fn from(d: pumpkin_util::BlockDirection) -> Self {
        match d {
            pumpkin_util::BlockDirection::Down => Self::Down,
            pumpkin_util::BlockDirection::Up => Self::Up,
            pumpkin_util::BlockDirection::North => Self::North,
            pumpkin_util::BlockDirection::South => Self::South,
            pumpkin_util::BlockDirection::West => Self::West,
            pumpkin_util::BlockDirection::East => Self::East,
        }
    }
}

impl From<BlockDirection> for pumpkin_util::BlockDirection {
    fn from(d: BlockDirection) -> Self {
        match d {
            BlockDirection::Down => Self::Down,
            BlockDirection::Up => Self::Up,
            BlockDirection::North => Self::North,
            BlockDirection::South => Self::South,
            BlockDirection::West => Self::West,
            BlockDirection::East => Self::East,
        }
    }
}

pub struct InvalidBlockFace;

impl TryFrom<i32> for BlockDirection {
    type Error = InvalidBlockFace;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Down),
            1 => Ok(Self::Up),
            2 => Ok(Self::North),
            3 => Ok(Self::South),
            4 => Ok(Self::West),
            5 => Ok(Self::East),
            _ => Err(InvalidBlockFace),
        }
    }
}

impl BlockDirection {
    #[must_use]
    pub const fn to_index(&self) -> u8 {
        match self {
            Self::Down => 0,
            Self::Up => 1,
            Self::North => 2,
            Self::South => 3,
            Self::West => 4,
            Self::East => 5,
        }
    }

    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::Down),
            1 => Some(Self::Up),
            2 => Some(Self::North),
            3 => Some(Self::South),
            4 => Some(Self::West),
            5 => Some(Self::East),
            _ => None,
        }
    }

    pub fn random(random: &mut RandomGenerator) -> Self {
        // Vanilla `Direction.getRandom` = `values[nextInt(values.length)]` over all six
        // directions, in declaration order (DOWN, UP, NORTH, SOUTH, WEST, EAST).
        Self::all()[random.next_bounded_i32(Self::all().len() as i32) as usize]
    }

    /// Vanilla `Direction.Plane.HORIZONTAL.getRandomDirection`: `nextInt(4)` over `[NORTH, EAST,
    /// SOUTH, WEST]`.
    ///
    /// The bound is the full length (a `len() - 1` here made the last direction unreachable) and
    /// the order is [`Self::horizontal_worldgen`], not [`Self::horizontal`], so a given draw picks
    /// the direction vanilla picks.
    pub fn random_horizontal(random: &mut RandomGenerator) -> HorizontalFacing {
        let directions = Self::horizontal_worldgen();
        directions[random.next_bounded_i32(directions.len() as i32) as usize]
    }

    #[must_use]
    pub fn by_index(index: usize) -> Option<Self> {
        Self::all().get(index % Self::all().len()).copied()
    }

    #[must_use]
    pub fn to_offset(&self) -> Vector3<i32> {
        match self {
            Self::Down => (0, -1, 0),
            Self::Up => (0, 1, 0),
            Self::North => (0, 0, -1),
            Self::South => (0, 0, 1),
            Self::West => (-1, 0, 0),
            Self::East => (1, 0, 0),
        }
        .into()
    }

    /// Returns the nearest BlockDirection to the given 3D direction vector, matching vanilla Direction.getApproximateNearest.
    #[must_use]
    pub fn get_nearest(x: f64, y: f64, z: f64) -> Self {
        let mut best_dir = Self::North;
        let mut max_dot = f64::NEG_INFINITY;
        for dir in Self::all() {
            let offset = dir.to_offset();
            let dot = x * f64::from(offset.x) + y * f64::from(offset.y) + z * f64::from(offset.z);
            if dot > max_dot {
                max_dot = dot;
                best_dir = dir;
            }
        }
        best_dir
    }

    #[must_use]
    pub const fn opposite(&self) -> Self {
        match self {
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::West => Self::East,
            Self::East => Self::West,
        }
    }

    #[must_use]
    pub const fn positive(&self) -> bool {
        matches!(self, Self::South | Self::East | Self::Up)
    }

    #[must_use]
    pub const fn all() -> [Self; 6] {
        [
            Self::Down,
            Self::Up,
            Self::North,
            Self::South,
            Self::West,
            Self::East,
        ]
    }
    #[must_use]
    pub const fn update_order() -> [Self; 6] {
        [
            Self::West,
            Self::East,
            Self::Down,
            Self::Up,
            Self::North,
            Self::South,
        ]
    }

    #[must_use]
    pub const fn abstract_block_update_order() -> [Self; 6] {
        [
            Self::West,
            Self::East,
            Self::North,
            Self::South,
            Self::Down,
            Self::Up,
        ]
    }

    #[must_use]
    pub const fn horizontal() -> [HorizontalFacing; 4] {
        [
            HorizontalFacing::North,
            HorizontalFacing::South,
            HorizontalFacing::West,
            HorizontalFacing::East,
        ]
    }

    /// The four horizontal directions in vanilla `Direction.Plane.HORIZONTAL` order NORTH, EAST,
    /// SOUTH, WEST (matching vanilla's `getRandomDirection(random)` indexes with `nextInt(4)`).
    /// Worldgen-parity code that samples or iterates a random horizontal direction MUST use this,
    /// not [`Self::horizontal`] (whose `[North, South, West, East]` order would pick a different
    /// direction for the same draw).
    #[must_use]
    pub const fn horizontal_worldgen() -> [HorizontalFacing; 4] {
        [
            HorizontalFacing::North,
            HorizontalFacing::East,
            HorizontalFacing::South,
            HorizontalFacing::West,
        ]
    }

    #[must_use]
    pub const fn flow_directions() -> [Self; 5] {
        [Self::Down, Self::North, Self::South, Self::West, Self::East]
    }

    #[must_use]
    pub const fn is_horizontal(&self) -> bool {
        matches!(self, Self::North | Self::South | Self::West | Self::East)
    }

    #[must_use]
    pub const fn vertical() -> [Self; 2] {
        [Self::Down, Self::Up]
    }

    #[must_use]
    pub const fn to_horizontal_facing(&self) -> Option<HorizontalFacing> {
        match self {
            Self::North => Some(HorizontalFacing::North),
            Self::South => Some(HorizontalFacing::South),
            Self::West => Some(HorizontalFacing::West),
            Self::East => Some(HorizontalFacing::East),
            _ => None,
        }
    }

    #[must_use]
    pub const fn to_horizontal_axis(&self) -> Option<HorizontalAxis> {
        match self {
            Self::North | Self::South => Some(HorizontalAxis::Z),
            Self::West | Self::East => Some(HorizontalAxis::X),
            _ => None,
        }
    }

    #[must_use]
    pub const fn to_cardinal_direction(&self) -> HorizontalFacing {
        match self {
            Self::South => HorizontalFacing::South,
            Self::West => HorizontalFacing::West,
            Self::East => HorizontalFacing::East,
            _ => HorizontalFacing::North,
        }
    }

    #[must_use]
    pub const fn from_cardinal_direction(direction: HorizontalFacing) -> Self {
        match direction {
            HorizontalFacing::North => Self::North,
            HorizontalFacing::South => Self::South,
            HorizontalFacing::West => Self::West,
            HorizontalFacing::East => Self::East,
        }
    }
    #[must_use]
    pub const fn to_axis(&self) -> Axis {
        match self {
            Self::North | Self::South => Axis::Z,
            Self::West | Self::East => Axis::X,
            Self::Up | Self::Down => Axis::Y,
        }
    }

    #[must_use]
    pub const fn to_facing(&self) -> Facing {
        match self {
            Self::North => Facing::North,
            Self::South => Facing::South,
            Self::West => Facing::West,
            Self::East => Facing::East,
            Self::Up => Facing::Up,
            Self::Down => Facing::Down,
        }
    }

    #[must_use]
    pub const fn rotate_clockwise(&self) -> Self {
        match self {
            Self::East => Self::South,
            Self::West => Self::North,
            Self::Up | Self::North => Self::East,
            Self::Down | Self::South => Self::West,
        }
    }

    #[must_use]
    pub const fn rotate_counter_clockwise(&self) -> Self {
        match self {
            Self::West => Self::South,
            Self::East => Self::North,
            Self::Up | Self::North => Self::West,
            Self::Down | Self::South => Self::East,
        }
    }
}

pub trait FacingExt {
    fn to_block_direction(&self) -> BlockDirection;
    fn to_horizontal_facing(&self) -> Option<HorizontalFacing>;
}

impl FacingExt for Facing {
    fn to_block_direction(&self) -> BlockDirection {
        match self {
            Self::North => BlockDirection::North,
            Self::South => BlockDirection::South,
            Self::West => BlockDirection::West,
            Self::East => BlockDirection::East,
            Self::Up => BlockDirection::Up,
            Self::Down => BlockDirection::Down,
        }
    }
    fn to_horizontal_facing(&self) -> Option<HorizontalFacing> {
        match self {
            Self::North => Some(HorizontalFacing::North),
            Self::South => Some(HorizontalFacing::South),
            Self::West => Some(HorizontalFacing::West),
            Self::East => Some(HorizontalFacing::East),
            _ => None,
        }
    }
}

pub trait HorizontalFacingExt {
    fn to_block_direction(&self) -> BlockDirection;
}

impl HorizontalFacingExt for HorizontalFacing {
    fn to_block_direction(&self) -> BlockDirection {
        match self {
            Self::North => BlockDirection::North,
            Self::South => BlockDirection::South,
            Self::West => BlockDirection::West,
            Self::East => BlockDirection::East,
        }
    }
}
