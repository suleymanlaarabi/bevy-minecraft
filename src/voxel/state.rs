//! Validated native states and shape metadata shared by rendering and gameplay.
use super::VoxelKind;
use bevy::prelude::*;
use pumpkin_data::{BlockState, BlockStateId, Property, block_properties::COLLISION_SHAPES};
use pumpkin_util::math::{boundingbox::BoundingBox, position::BlockPos};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VoxelState(BlockStateId);

#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockBox {
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
    pub(crate) fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
    pub(crate) fn size(self) -> Vec3 {
        self.max - self.min
    }
    pub(crate) fn translated(self, delta: Vec3) -> Self {
        Self {
            min: self.min + delta,
            max: self.max + delta,
        }
    }
    pub(crate) fn ray(self, origin: Vec3, direction: Vec3) -> Option<(f32, Vec3)> {
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
        VoxelKind(self.0.to_block().id)
    }
    pub(crate) const fn native(self) -> &'static BlockState {
        self.0.to_state()
    }
    pub(crate) const fn is_opaque_cube(self) -> bool {
        self.native().is_full_cube() && self.native().opacity == 15
    }
    pub(crate) fn has_water(self) -> bool {
        static WATER: std::sync::OnceLock<Box<[bool]>> = std::sync::OnceLock::new();
        WATER.get_or_init(|| {
            (0..=u16::MAX)
                .map(|raw| {
                    let Some(id) = BlockStateId::new(raw) else {
                        return false;
                    };
                    let kind = VoxelKind(id.to_block().id);
                    kind == VoxelKind::Water
                        || id.is_waterlogged()
                        || matches!(
                            kind.name(),
                            "bubble_column" | "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant"
                        )
                })
                .collect()
        })[self.id() as usize]
    }
    fn offset_limits(self) -> (f32, f32) {
        static OFFSETS: std::sync::OnceLock<Box<[(f32, f32)]>> = std::sync::OnceLock::new();
        OFFSETS.get_or_init(|| {
            VoxelKind::ALL
                .iter()
                .map(|kind| {
                    let state = kind.default_state();
                    let Some(&shape) = state.native().outline_shapes.first() else {
                        return (0.0, 0.0);
                    };
                    let base = BlockBox::from(COLLISION_SHAPES[shape as usize]).min;
                    let samples = [(1, 0), (7, 3), (-18, -7)].map(|(x, z)| {
                        let pos = BlockPos::new(x, 0, z);
                        let shifted = state
                            .native()
                            .get_block_outline_shapes_at(&pos)
                            .next()
                            .map(BlockBox::from)
                            .unwrap();
                        let seed = pumpkin_util::random::hash_block_pos(x, 0, z) as u64;
                        (seed, shifted.min - base)
                    });
                    let horizontal = [0.0f32, 0.125, 0.25]
                        .into_iter()
                        .find(|&limit| {
                            samples.iter().all(|&(seed, delta)| {
                                let x = (((seed & 15) as f32 / 15.0 - 0.5) * 0.5)
                                    .clamp(-limit, limit)
                                    + limit;
                                let z = (((((seed >> 8) & 15) as f32 / 15.0) - 0.5) * 0.5)
                                    .clamp(-limit, limit)
                                    + limit;
                                (x - delta.x).abs() < 1e-5 && (z - delta.z).abs() < 1e-5
                            })
                        })
                        .unwrap_or(0.0);
                    let vertical = samples
                        .iter()
                        .find_map(|&(seed, delta)| {
                            let fraction = ((seed >> 4) & 15) as f32 / 15.0;
                            (fraction > 0.0).then(|| (delta.y / fraction * 1000.0).round() / 1000.0)
                        })
                        .unwrap_or(0.0);
                    (horizontal, vertical)
                })
                .collect()
        })[self.kind().index()]
    }
    /// Translation of resource-pack models, which are unshifted block-local geometry.
    pub(crate) fn visual_offset(self, world: IVec3) -> Vec3 {
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
    pub(crate) fn properties(self) -> Vec<Property> {
        self.0
            .to_block()
            .properties(self.0)
            .map_or_else(Vec::new, |props| props.to_props())
    }
    pub fn property(self, key: &str) -> Option<&'static str> {
        self.properties()
            .iter()
            .find(|p| p.key_str() == key)
            .map(|p| p.value_str())
    }
    pub(crate) fn with(self, edits: &[(&str, &str)]) -> Self {
        let mut props = self.properties();
        for &(key, value) in edits {
            if let Some(prop) = props.iter_mut().find(|p| p.key_str() == key)
                && let Some(next) = Property::from_str_pair(key, value)
            {
                *prop = next;
            }
        }
        Self(
            self.0
                .to_block()
                .from_properties(&props)
                .to_state_id(self.0.to_block()),
        )
    }
    pub(crate) fn collision_boxes(self, world: IVec3) -> impl Iterator<Item = BlockBox> {
        let delta = self.shape_offset(world);
        self.native()
            .collision_shapes
            .iter()
            .map(move |&id| BlockBox::from(COLLISION_SHAPES[id as usize]).translated(delta))
    }
    /// Selection excludes the fluid shape that Pumpkin adds to its outline iterator.
    pub(crate) fn selection_boxes(self, world: IVec3) -> impl Iterator<Item = BlockBox> {
        let delta = self.shape_offset(world);
        self.native()
            .outline_shapes
            .iter()
            .map(move |&id| BlockBox::from(COLLISION_SHAPES[id as usize]).translated(delta))
    }
    /// Pumpkin's shapes were extracted at the origin, whose vanilla offset is
    /// `(-h, -v, -h)` (its position hash is zero): undo that, then apply this
    /// position's offset. `offset_limits` relies on the same relation.
    fn shape_offset(self, world: IVec3) -> Vec3 {
        let (horizontal, vertical) = self.offset_limits();
        self.visual_offset(world) + Vec3::new(horizontal, vertical, horizontal)
    }
    pub(crate) fn full_collision(self) -> bool {
        static FULL: std::sync::OnceLock<Box<[bool]>> = std::sync::OnceLock::new();
        FULL.get_or_init(|| {
            (0..=u16::MAX)
                .map(|id| {
                    let Some(state) = Self::from_id(id) else {
                        return false;
                    };
                    let shapes = state.native().collision_shapes;
                    shapes.len() == 1 && {
                        let b = BlockBox::from(COLLISION_SHAPES[shapes[0] as usize]);
                        b.min == Vec3::ZERO && b.max == Vec3::ONE
                    }
                })
                .collect()
        })[self.id() as usize]
    }
    pub(crate) fn replaceable(self) -> bool {
        self.native().replaceable()
    }
    pub(crate) fn removed(self) -> Self {
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
