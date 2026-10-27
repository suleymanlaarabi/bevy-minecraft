//! Validated native states and shape metadata shared by rendering and gameplay.
use super::VoxelKind;
use bevy_math::prelude::*;
use pumpkin_data::{BlockState, BlockStateId, Property, PropertyKey, block_properties::Facing};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VoxelState(hollow_core::StateId);

pub use hollow_core::shape::BlockBox;

impl VoxelState {
    pub fn from_id(id: impl Into<u32>) -> Option<Self> {
        let id = id.into();
        (id < Self::count() as u32).then_some(Self(hollow_core::StateId(id)))
    }
    pub const fn id(self) -> u32 {
        self.0.0
    }
    pub(crate) fn native_id(self) -> BlockStateId {
        BlockStateId::new(u16::try_from(self.id()).expect("Minecraft state ID"))
            .expect("Registered Minecraft state")
    }
    pub fn kind(self) -> VoxelKind {
        VoxelKind(self.definition().block.0)
    }
    pub fn native(self) -> &'static BlockState {
        self.native_id().to_state()
    }
    pub fn is_opaque_cube(self) -> bool {
        self.definition().is_full_cube() && self.definition().opacity == 15
    }
    fn offset_limits(self) -> (f32, f32) {
        if !self.is_native() {
            return (0.0, 0.0);
        }
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
    pub fn properties(self) -> impl Iterator<Item = Property> + Clone {
        self.is_native()
            .then(|| self.native_id())
            .into_iter()
            .flat_map(|id| id.properties())
    }
    pub fn property(self, key: PropertyKey) -> Option<Property> {
        self.is_native()
            .then(|| self.native_id().property(key))
            .flatten()
    }
    pub fn supports(self, property: Property) -> bool {
        self.is_native() && self.native_id().supports(property)
    }
    pub fn with(self, edits: &[Property]) -> Self {
        assert!(
            self.is_native(),
            "Native properties only; custom content uses typed PropertyId"
        );
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
        let shapes = super::content::catalog().shapes(self.definition().collision);
        shapes
            .iter()
            .copied()
            .map(move |bounds| bounds.translated(delta))
    }
    /// Selection excludes the fluid shape that Pumpkin adds to its outline iterator.
    pub fn selection_boxes(self, world: IVec3) -> impl Iterator<Item = BlockBox> {
        let delta = self.shape_offset(world);
        let shapes = super::content::catalog().shapes(self.definition().selection);
        shapes
            .iter()
            .copied()
            .map(move |bounds| bounds.translated(delta))
    }
    /// Pumpkin's shapes were extracted at the origin, whose vanilla offset is
    /// `(-h, -v, -h)` (its position hash is zero): undo that, then apply this
    /// position's offset using the native registry limits.
    pub fn shape_offset(self, world: IVec3) -> Vec3 {
        let (horizontal, vertical) = self.offset_limits();
        self.visual_offset(world) + Vec3::new(horizontal, vertical, horizontal)
    }
    pub fn full_collision(self) -> bool {
        self.definition().full_collision
    }
    pub fn replaceable(self) -> bool {
        self.definition().replaceable()
    }
}
impl VoxelKind {
    pub fn default_state(self) -> VoxelState {
        VoxelState(hollow_core::StateId(if self.is_native() {
            pumpkin_data::BlockId::new_or_air(self.0 as u16)
                .to_block()
                .default_state
                .id
                .as_u16() as u32
        } else {
            super::content::catalog()
                .block(hollow_core::BlockId(self.0))
                .default_state
                .0
        }))
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
        self.is_liquid()
    }
    fn is_air(self) -> bool {
        self.is_air()
    }
    fn is_opaque(self) -> bool {
        self.is_opaque_cube()
    }
}

impl VoxelState {
    pub const AIR: Self = Self(hollow_core::StateId(0));
    pub fn count() -> usize {
        super::content::try_catalog().map_or(BlockStateId::COUNT as usize, |c| c.state_count())
    }
    pub fn is_native(self) -> bool {
        self.id() < u32::from(BlockStateId::COUNT)
    }
    #[inline]
    pub fn definition(self) -> &'static hollow_core::content::StateDefinition {
        super::content::catalog().state(self.0)
    }
    pub fn is_air(self) -> bool {
        self.definition().air
    }
    pub fn is_liquid(self) -> bool {
        self.definition().fluid.id != hollow_core::FluidId::NONE
    }
    pub fn with_fluid(self, fluid: hollow_core::FluidId) -> Option<Self> {
        super::content::catalog()
            .container(self.0, fluid)
            .and_then(|rule| Self::from_id(rule.filled.0))
    }
    pub fn shape_depends_on_position(self) -> bool {
        self.offset_limits() != (0.0, 0.0)
    }
    pub fn collision_empty(self) -> bool {
        super::content::catalog()
            .shapes(self.definition().collision)
            .is_empty()
    }
    pub fn selection_empty(self) -> bool {
        super::content::catalog()
            .shapes(self.definition().selection)
            .is_empty()
    }
    pub fn is_side_solid(self, side: pumpkin_data::BlockDirection) -> bool {
        let index = side.to_index();
        self.definition().solid_faces & (1 << index) != 0
    }
    pub fn is_center_solid(self, side: pumpkin_data::BlockDirection) -> bool {
        let index = side.to_index();
        self.definition().center_faces & (1 << index) != 0
    }
    pub fn with_property(
        self,
        property: hollow_core::content::PropertyId,
        value: u16,
    ) -> Option<Self> {
        super::content::catalog()
            .with(self.0, property, value)
            .map(Self)
    }
}
