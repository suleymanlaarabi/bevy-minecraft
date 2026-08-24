use bevy::prelude::*;
use std::sync::Arc;
pub(crate) const MESH_CHANGED: u8 = 1;
pub(crate) const COLLIDER_CHANGED: u8 = 2;
type VoxelBuffer = Vec<VoxelKind>;
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VoxelKind {
    #[default]
    Air,
    Grass,
    Dirt,
    Stone,
    Sand,
    Snow,
    Wood,
    Leaves,
    Water,
}
impl VoxelKind {
    pub const fn is_solid(self) -> bool {
        !matches!(self, Self::Air | Self::Leaves | Self::Water)
    }

    pub const fn is_liquid(self) -> bool {
        matches!(self, Self::Water)
    }

    pub(crate) const fn is_opaque(self) -> bool {
        !matches!(self, Self::Air | Self::Water)
    }

    pub(crate) const fn color(self) -> [f32; 4] {
        match self {
            Self::Air => [0.0; 4],
            Self::Grass => [0.25, 0.65, 0.18, 1.0],
            Self::Dirt => [0.40, 0.25, 0.12, 1.0],
            Self::Stone => [0.40, 0.40, 0.40, 1.0],
            Self::Sand => [0.76, 0.68, 0.42, 1.0],
            Self::Snow => [0.92, 0.96, 1.0, 1.0],
            Self::Wood => [0.36, 0.20, 0.08, 1.0],
            Self::Leaves => [0.12, 0.48, 0.10, 1.0],
            Self::Water => [0.08, 0.35, 0.72, 0.62],
        }
    }

    pub(crate) const fn texture_layer(self, axis: usize, positive: bool) -> u32 {
        match self {
            Self::Grass if axis == 1 && positive => 0,
            Self::Grass if axis == 1 => 2,
            Self::Grass => 1,
            Self::Wood if axis == 1 => 7,
            Self::Wood => 6,
            Self::Dirt => 2,
            Self::Stone => 3,
            Self::Sand => 4,
            Self::Snow => 5,
            Self::Leaves => 8,
            Self::Air | Self::Water => 0,
        }
    }

    pub(crate) const fn texture_tint(self, axis: usize, positive: bool) -> [f32; 3] {
        match (self, axis, positive) {
            (Self::Grass, 1, true) => [0.42, 0.72, 0.24],
            (Self::Leaves, _, _) => [0.20, 0.64, 0.16],
            _ => [1.0, 1.0, 1.0],
        }
    }
}
/// Dense chunk data. Cloning it is O(1); edits use copy-on-write while a build reads it.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels {
    pub(crate) size: i32,
    pub(crate) height: i32,
    cells: Arc<[VoxelKind]>,
    halo: Arc<[VoxelKind]>,
    pub(crate) changes: u8,
    pub(crate) modified: bool,
}
impl ChunkVoxels {
    pub(crate) fn generated(size: i32, height: i32, cells: VoxelBuffer, halo: VoxelBuffer) -> Self {
        Self {
            size,
            height,
            cells: cells.into(),
            halo: halo.into(),
            changes: MESH_CHANGED | COLLIDER_CHANGED,
            modified: false,
        }
    }

    pub fn get(&self, local: IVec3) -> Option<VoxelKind> {
        self.index(local).map(|index| self.cells[index])
    }

    pub(crate) fn sample(&self, local: IVec3) -> VoxelKind {
        self.get(local)
            .or_else(|| self.halo_kind(local))
            .unwrap_or(VoxelKind::Air)
    }

    /// Changes one cell. Out-of-bounds and identical values return `false`.
    pub fn set(&mut self, local: IVec3, kind: VoxelKind) -> bool {
        let Some(index) = self.index(local) else {
            return false;
        };
        let old = self.cells[index];
        if old == kind {
            return false;
        }
        Arc::make_mut(&mut self.cells)[index] = kind;
        self.changes |= MESH_CHANGED;
        if old.is_solid() != kind.is_solid() {
            self.changes |= COLLIDER_CHANGED;
        }
        self.modified = true;
        true
    }

    pub(crate) fn solid_positions(&self) -> Vec<IVec3> {
        let size = self.size as usize;
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.is_solid())
            .map(|(index, _)| {
                IVec3::new(
                    (index % size) as i32,
                    (index / (size * size)) as i32,
                    (index / size % size) as i32,
                )
            })
            .collect()
    }

    fn index(&self, local: IVec3) -> Option<usize> {
        if local.x < 0
            || local.x >= self.size
            || local.y < 0
            || local.y >= self.height
            || local.z < 0
            || local.z >= self.size
        {
            return None;
        }
        Some(
            local.x as usize
                + self.size as usize * (local.z as usize + self.size as usize * local.y as usize),
        )
    }

    fn halo_kind(&self, local: IVec3) -> Option<VoxelKind> {
        if !(0..self.height).contains(&local.y) {
            return None;
        }
        let size = self.size as usize;
        let (side, offset) = match (local.x, local.z) {
            (-1, z) if (0..self.size).contains(&z) => (0, z),
            (x, z) if x == self.size && (0..self.size).contains(&z) => (1, z),
            (x, -1) if (0..self.size).contains(&x) => (2, x),
            (x, z) if z == self.size && (0..self.size).contains(&x) => (3, x),
            _ => return None,
        };
        Some(self.halo[offset as usize + size * (side + 4 * local.y as usize)])
    }
}
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelChunk {
    pub position: IVec2,
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct VoxelViewer;

#[derive(Event, Clone, Copy, Debug)]
pub struct SetVoxel {
    pub world_position: IVec3,
    pub kind: VoxelKind,
}

impl SetVoxel {
    pub const fn new(world_position: IVec3, kind: VoxelKind) -> Self {
        Self {
            world_position,
            kind,
        }
    }
}
