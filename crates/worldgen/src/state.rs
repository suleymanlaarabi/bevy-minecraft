//! Validated native states and shape metadata shared by rendering and gameplay.
use super::VoxelKind;
use bevy_math::prelude::*;
use pumpkin_data::{
    BlockId, BlockState, BlockStateId, Property, PropertyKey, block_properties::COLLISION_SHAPES,
    block_properties::Facing,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VoxelState(hollow_core::StateId);

pub use hollow_core::shape::BlockBox;

impl VoxelState {
    pub fn from_id(id: impl Into<u32>) -> Option<Self> {
        let id = id.into();
        (id < u32::from(BlockStateId::COUNT)).then_some(Self(hollow_core::StateId(id)))
    }
    pub const fn id(self) -> u32 {
        self.0.0
    }
    pub(crate) fn native_id(self) -> BlockStateId {
        BlockStateId::new(u16::try_from(self.id()).expect("Minecraft state ID"))
            .expect("Registered Minecraft state")
    }
    pub fn kind(self) -> VoxelKind {
        if self.native().is_air() {
            VoxelKind::Air
        } else {
            VoxelKind(self.native_id().to_block_id())
        }
    }
    pub fn native(self) -> &'static BlockState {
        self.native_id().to_state()
    }
    pub fn is_opaque_cube(self) -> bool {
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
        self.native_id().properties()
    }
    pub fn property(self, key: PropertyKey) -> Option<Property> {
        self.native_id().property(key)
    }
    pub fn supports(self, property: Property) -> bool {
        self.native_id().supports(property)
    }
    pub fn with(self, edits: &[Property]) -> Self {
        let state = self
            .native_id()
            .with_properties(edits)
            .expect("Valid gameplay block properties");
        Self(hollow_core::StateId(u32::from(state.as_u16())))
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
            .map(move |&id| block_box(COLLISION_SHAPES[id as usize]).translated(delta))
    }
    /// Selection excludes the fluid shape that Pumpkin adds to its outline iterator.
    pub fn selection_boxes(self, world: IVec3) -> impl Iterator<Item = BlockBox> {
        let delta = self.shape_offset(world);
        self.native()
            .outline_shapes
            .iter()
            .map(move |&id| block_box(COLLISION_SHAPES[id as usize]).translated(delta))
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
                        let bounds = block_box(COLLISION_SHAPES[shapes[0] as usize]);
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
        VoxelState(hollow_core::StateId(
            self.block().default_state.id.as_u16() as u32
        ))
    }
}

const _: () = assert!(std::mem::size_of::<VoxelState>() == 4);

pub fn block_box(bounds: pumpkin_util::math::boundingbox::BoundingBox) -> BlockBox {
    BlockBox {
        min: Vec3::new(
            bounds.min.x as f32,
            bounds.min.y as f32,
            bounds.min.z as f32,
        ),
        max: Vec3::new(
            bounds.max.x as f32,
            bounds.max.y as f32,
            bounds.max.z as f32,
        ),
    }
}

impl hollow_core::VoxelCell for VoxelState {
    type Kind = VoxelKind;
    fn id(self) -> u32 {
        self.id()
    }
    fn from_id(id: u32) -> Option<Self> {
        Self::from_id(id)
    }
    fn kind(self) -> VoxelKind {
        self.kind()
    }
    fn default_state(kind: VoxelKind) -> Self {
        kind.default_state()
    }
    fn air() -> Self {
        VoxelKind::Air.default_state()
    }
    fn is_liquid(self) -> bool {
        self.has_water() || self.kind() == VoxelKind::Lava
    }
    fn is_air(self) -> bool {
        self.native().is_air()
    }
    fn is_opaque(self) -> bool {
        self.is_opaque_cube()
    }
}
