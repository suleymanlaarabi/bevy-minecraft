use bevy::prelude::*;
use std::sync::Arc;
pub(crate) const MESH_CHANGED: u8 = 1;
pub(crate) const COLLIDER_CHANGED: u8 = 2;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VoxelKind {
    #[default]
    Air,
    Grass,
    Dirt,
    Stone,
}
impl VoxelKind {
    pub const fn is_solid(self) -> bool {
        !matches!(self, Self::Air)
    }

    pub(crate) const fn color(self) -> [f32; 4] {
        match self {
            Self::Air => [0.0; 4],
            Self::Grass => [0.25, 0.65, 0.18, 1.0],
            Self::Dirt => [0.40, 0.25, 0.12, 1.0],
            Self::Stone => [0.40, 0.40, 0.40, 1.0],
        }
    }

    pub(crate) const fn terrain_at(y: i32, surface: i32) -> Self {
        if y < 0 || y >= surface {
            Self::Air
        } else if y == surface - 1 {
            Self::Grass
        } else if y >= surface - 4 {
            Self::Dirt
        } else {
            Self::Stone
        }
    }
}
/// Dense chunk data. Cloning it is O(1); edits use copy-on-write while a build reads it.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels {
    pub(crate) size: i32,
    pub(crate) height: i32,
    cells: Arc<[VoxelKind]>,
    halo: Arc<[i32]>,
    pub(crate) changes: u8,
    pub(crate) modified: bool,
}
impl ChunkVoxels {
    pub(crate) fn generated(size: i32, height: i32, cells: Vec<VoxelKind>, halo: Vec<i32>) -> Self {
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
        self.get(local).unwrap_or_else(|| {
            self.halo_height(local).map_or(VoxelKind::Air, |surface| {
                VoxelKind::terrain_at(local.y, surface)
            })
        })
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

    fn halo_height(&self, local: IVec3) -> Option<i32> {
        let size = self.size as usize;
        let index = match (local.x, local.z) {
            (-1, z) if (0..self.size).contains(&z) => z as usize,
            (x, z) if x == self.size && (0..self.size).contains(&z) => size + z as usize,
            (x, -1) if (0..self.size).contains(&x) => size * 2 + x as usize,
            (x, z) if z == self.size && (0..self.size).contains(&x) => size * 3 + x as usize,
            _ => return None,
        };
        Some(self.halo[index])
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
