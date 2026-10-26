//! Validated native states and shape metadata shared by rendering and gameplay.
use super::VoxelKind;
use bevy_math::prelude::*;
use pumpkin_data::{
    BlockId, BlockState, BlockStateId, Property, PropertyKey, block_properties::COLLISION_SHAPES,
    block_properties::Facing,
};
use pumpkin_util::math::boundingbox::BoundingBox;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VoxelState(BlockStateId);

#[derive(Clone, Copy, Debug)]
pub struct BlockBox {
    pub min: Vec3,
    pub max: Vec3,
}
impl From<BoundingBox> for BlockBox {
    fn from(b: BoundingBox) -> Self {
        Self {
            min: Vec3::new(b.min.x as f32, b.min.y as f32, b.min.z as f32),
            max: Vec3::new(b.max.x as f32, b.max.y as f32, b.max.z as f32),
        }
    }
}
impl BlockBox {
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub fn size(self) -> Vec3 {
        self.max - self.min
    }
    pub fn translated(self, delta: Vec3) -> Self {
        Self {
            min: self.min + delta,
            max: self.max + delta,
        }
    }
    pub fn ray(self, origin: Vec3, direction: Vec3) -> Option<(f32, Vec3)> {
        let mut enter = 0.0f32;
        let mut exit = f32::INFINITY;
        let axis =
            if direction.x.abs() >= direction.y.abs() && direction.x.abs() >= direction.z.abs() {
                0
            } else if direction.y.abs() >= direction.z.abs() {
                1
            } else {
                2
            };
        let mut normal = Vec3::ZERO;
        normal[axis] = -direction[axis].signum();
        for axis in 0..3 {
            if direction[axis].abs() < 1e-8 {
                if origin[axis] < self.min[axis] || origin[axis] > self.max[axis] {
                    return None;
                }
                continue;
            }
            let a = (self.min[axis] - origin[axis]) / direction[axis];
            let b = (self.max[axis] - origin[axis]) / direction[axis];
            let near = a.min(b);
            if near > enter {
                enter = near;
                normal = Vec3::ZERO;
                normal[axis] = -direction[axis].signum();
            }
            exit = exit.min(a.max(b));
            if enter > exit {
                return None;
            }
        }
        (exit >= 0.0).then_some((enter, normal))
    }
}

impl VoxelState {
    pub const fn from_id(id: u16) -> Option<Self> {
        match BlockStateId::new(id) {
            Some(id) => Some(Self(id)),
            None => None,
        }
    }
    pub const fn id(self) -> u16 {
        self.0.as_u16()
    }
    pub const fn kind(self) -> VoxelKind {
        if self.native().is_air() {
            VoxelKind::Air
        } else {
            VoxelKind(self.0.to_block_id())
        }
    }
    pub const fn native(self) -> &'static BlockState {
        self.0.to_state()
    }
    pub const fn is_opaque_cube(self) -> bool {
        self.native().is_full_cube() && self.native().opacity == 15
    }
    pub fn has_water(self) -> bool {
        self.flags() & 1 != 0
    }
    fn offset_limits(self) -> (f32, f32) {
        static OFFSETS: std::sync::LazyLock<Box<[(f32, f32)]>> = std::sync::LazyLock::new(|| {
            VoxelKind::ALL
                .iter()
                .map(|kind| kind.block().shape_offset_limits())
                .collect()
        });
        OFFSETS[self.kind().index()]
    }
    /// Translation of resource-pack models, which are unshifted block-local geometry.
    pub fn visual_offset(self, world: IVec3) -> Vec3 {
        let (horizontal, vertical) = self.offset_limits();
        if horizontal == 0.0 && vertical == 0.0 {
            return Vec3::ZERO;
        }
        let seed = pumpkin_util::random::hash_block_pos(world.x, 0, world.z) as u64;
        let x = (((seed & 15) as f32 / 15.0 - 0.5) * 0.5).clamp(-horizontal, horizontal);
        let z = (((((seed >> 8) & 15) as f32 / 15.0) - 0.5) * 0.5).clamp(-horizontal, horizontal);
        let y = (((seed >> 4) & 15) as f32 / 15.0 - 1.0) * vertical;
        Vec3::new(x, y, z)
    }
    pub fn properties(self) -> impl ExactSizeIterator<Item = Property> + Clone {
        self.0.properties()
    }
    pub fn property(self, key: PropertyKey) -> Option<Property> {
        self.0.property(key)
    }
    pub fn supports(self, property: Property) -> bool {
        self.0.supports(property)
    }
    pub fn with(self, edits: &[Property]) -> Self {
        Self(
            self.0
                .with_properties(edits)
                .expect("Valid gameplay block properties"),
        )
    }
    pub fn facing(self) -> Facing {
        match self.property(PropertyKey::Facing) {
            Some(Property::Facing(facing)) => facing,
            _ => Facing::North,
        }
    }
    pub fn attachment(direction: Facing, value: bool) -> Property {
        match direction {
            Facing::North => Property::North(value),
            Facing::East => Property::East(value),
            Facing::South => Property::South(value),
            Facing::West => Property::West(value),
            Facing::Up => Property::Up(value),
            Facing::Down => Property::Down(value),
        }
    }
    pub fn collision_boxes(self, world: IVec3) -> impl Iterator<Item = BlockBox> {
        let delta = self.shape_offset(world);
        self.native()
            .collision_shapes
            .iter()
            .map(move |&id| BlockBox::from(COLLISION_SHAPES[id as usize]).translated(delta))
    }
    /// Selection excludes the fluid shape that Pumpkin adds to its outline iterator.
    pub fn selection_boxes(self, world: IVec3) -> impl Iterator<Item = BlockBox> {
        let delta = self.shape_offset(world);
        self.native()
            .outline_shapes
            .iter()
            .map(move |&id| BlockBox::from(COLLISION_SHAPES[id as usize]).translated(delta))
    }
    /// Pumpkin's shapes were extracted at the origin, whose vanilla offset is
    /// `(-h, -v, -h)` (its position hash is zero): undo that, then apply this
    /// position's offset using the native registry limits.
    fn shape_offset(self, world: IVec3) -> Vec3 {
        let (horizontal, vertical) = self.offset_limits();
        self.visual_offset(world) + Vec3::new(horizontal, vertical, horizontal)
    }
    fn flags(self) -> u8 {
        static FLAGS: std::sync::LazyLock<Box<[u8]>> = std::sync::LazyLock::new(|| {
            (0..BlockStateId::COUNT)
                .map(|id| {
                    let state = BlockStateId::new(id).expect("Native registry state");
                    let water = matches!(
                        state.to_block_id(),
                        BlockId::WATER
                            | BlockId::BUBBLE_COLUMN
                            | BlockId::SEAGRASS
                            | BlockId::TALL_SEAGRASS
                            | BlockId::KELP
                            | BlockId::KELP_PLANT
                    ) || state.is_waterlogged();
                    let shapes = state.to_state().collision_shapes;
                    let full = shapes.len() == 1 && {
                        let bounds = BlockBox::from(COLLISION_SHAPES[shapes[0] as usize]);
                        bounds.min == Vec3::ZERO && bounds.max == Vec3::ONE
                    };
                    u8::from(water) | (u8::from(full) << 1)
                })
                .collect()
        });
        FLAGS[self.id() as usize]
    }
    pub fn full_collision(self) -> bool {
        self.flags() & 2 != 0
    }
    pub fn replaceable(self) -> bool {
        self.native().replaceable()
    }
    pub fn removed(self) -> Self {
        if self.has_water() {
            VoxelKind::Water.default_state()
        } else {
            VoxelKind::Air.default_state()
        }
    }
}
impl VoxelKind {
    pub const fn default_state(self) -> VoxelState {
        VoxelState(self.block().default_state.id)
    }
}

const _: () = assert!(std::mem::size_of::<VoxelState>() == 2);

/// Identity of the registry's ordered block-state mapping.
pub fn registry_identity() -> u64 {
    static ID: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
        let mut hash = 0xcbf29ce484222325u64;
        for id in 0..BlockStateId::COUNT {
            let state = BlockStateId::new(id).unwrap();
            let key = format!(
                "{id}:{}:{:?}",
                state.to_block().name,
                state.properties().collect::<Vec<_>>()
            );
            for byte in key.bytes().chain(std::iter::once(0)) {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
        hash
    });
    *ID
}
