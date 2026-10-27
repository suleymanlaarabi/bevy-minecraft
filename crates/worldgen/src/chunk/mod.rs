//! Shared generated column data; dense working buffers belong to ProtoChunk.
use crate::{ChunkSnapshot, VoxelState, sections::Layer};
use format::LightContainer;
use pumpkin_data::block_properties::blocks_movement;
use pumpkin_data::{BlockState, BlockStateId, tag::Block::MINECRAFT_LEAVES};
use std::sync::{Arc, Mutex, RwLock};
pub mod format;
pub mod io;
pub const CHUNK_AREA: usize = 256;

pub struct ChunkData {
    pub fluid_ticks: Arc<[crate::embedding::GeneratedFluidTick]>,
    pub x: i32,
    pub z: i32,
    pub section: ChunkSections,
    pub heightmap: Mutex<ChunkHeightmaps>,
    pub light: Mutex<Layer<u8>>,
}
pub struct ChunkSections {
    pub count: usize,
    pub min_y: i32,
    pub states: RwLock<Layer<VoxelState>>,
    pub biomes: Arc<[u8]>,
}
impl ChunkSections {
    pub fn get_block_absolute_y(&self, x: usize, y: i32, z: usize) -> Option<BlockStateId> {
        if x >= 16 || z >= 16 || y < self.min_y || y >= self.min_y + self.count as i32 * 16 {
            return None;
        }
        Some(
            BlockStateId::new(
                self.states
                    .read()
                    .unwrap()
                    .get((y - self.min_y) as usize * 256 + z * 16 + x)
                    .id(),
            )
            .unwrap(),
        )
    }
    pub fn get_rough_biome_absolute_y(&self, x: usize, y: i32, z: usize) -> Option<u8> {
        let quart_height = self.count * 4;
        let quart_y = (y - self.min_y) >> 2;
        if quart_y < 0 || quart_y as usize >= quart_height || x >= 16 || z >= 16 {
            return None;
        }
        self.biomes
            .get(quart_height * 4 * (x >> 2) + 4 * quart_y as usize + (z >> 2))
            .copied()
    }
}
impl ChunkData {
    pub fn snapshot(&self) -> ChunkSnapshot {
        ChunkSnapshot {
            fluid_ticks: self.fluid_ticks.clone(),
            chunk_x: self.x,
            chunk_z: self.z,
            min_y: self.section.min_y,
            height: self.section.count as i32 * 16,
            states: self.section.states.read().unwrap().clone(),
            light: self.light.lock().unwrap().clone(),
        }
    }
    pub fn set_block_absolute_y(&self, x: usize, y: i32, z: usize, state: BlockStateId) {
        if x < 16
            && z < 16
            && y >= self.section.min_y
            && y < self.section.min_y + self.section.count as i32 * 16
        {
            let changed = self.section.states.write().unwrap().set(
                (y - self.section.min_y) as usize * 256 + z * 16 + x,
                VoxelState::from_id(state.as_u16()).unwrap(),
            );
            if changed {
                let mut heightmaps = self.heightmap.lock().unwrap();
                for kind in [
                    ChunkHeightmapType::WorldSurface,
                    ChunkHeightmapType::MotionBlocking,
                    ChunkHeightmapType::MotionBlockingNoLeaves,
                ] {
                    heightmaps.update(
                        kind,
                        x as i32,
                        y,
                        z as i32,
                        state.to_state(),
                        self.section.min_y,
                        |height| {
                            self.section
                                .get_block_absolute_y(x, height, z)
                                .unwrap_or(BlockStateId::AIR)
                                .to_state()
                        },
                    );
                }
            }
        }
    }
}

#[derive(Default, Clone)]
pub struct ChunkLight {
    pub sky_light: Box<[LightContainer]>,
    pub block_light: Box<[LightContainer]>,
}

#[derive(Debug, Clone, Copy)]
pub enum ChunkHeightmapType {
    WorldSurface = 0,
    MotionBlocking = 1,
    MotionBlockingNoLeaves = 2,
}
impl TryFrom<usize> for ChunkHeightmapType {
    type Error = &'static str;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::WorldSurface),
            1 => Ok(Self::MotionBlocking),
            2 => Ok(Self::MotionBlockingNoLeaves),
            _ => Err("Invalid usize value for ChunkHeightmapType. The value should be 0~2."),
        }
    }
}

impl ChunkHeightmapType {
    #[must_use]
    pub fn is_opaque(&self, block_state: &BlockState) -> bool {
        let block = block_state.id.to_block_id();
        match self {
            Self::WorldSurface => !block_state.is_air(),
            Self::MotionBlocking => blocks_movement(block_state, block) || block_state.is_liquid(),
            Self::MotionBlockingNoLeaves => {
                (blocks_movement(block_state, block) || block_state.is_liquid())
                    && !block.has_tag(MINECRAFT_LEAVES)
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkHeightmaps {
    pub world_surface: Option<Box<[i64]>>,
    pub motion_blocking: Option<Box<[i64]>>,
    pub motion_blocking_no_leaves: Option<Box<[i64]>>,
}

impl ChunkHeightmaps {
    /// Longs a heightmap occupies: 256 columns of 9 bits, 7 columns per long.
    pub const LONGS: usize = 37;

    pub fn set(&mut self, heightmap: ChunkHeightmapType, x: i32, z: i32, height: i32, min_y: i32) {
        let data = match heightmap {
            ChunkHeightmapType::WorldSurface => &mut self.world_surface,
            ChunkHeightmapType::MotionBlocking => &mut self.motion_blocking,
            ChunkHeightmapType::MotionBlockingNoLeaves => &mut self.motion_blocking_no_leaves,
        };

        let data = data.get_or_insert_with(|| vec![0; Self::LONGS].into_boxed_slice());

        let local_x = (x & 15) as usize;
        let local_z = (z & 15) as usize;
        let column_idx = local_z * 16 + local_x;

        // In Minecraft 1.16+, height is stored as (y - min_y + 1). 0 means below min_y.
        // It uses 9 bits per value, packed such that they do not cross u64 boundaries.
        // 64 / 9 = 7 values per u64.
        let val = (height - min_y + 1).max(0) as u64;

        let array_idx = column_idx / 7;
        let shift = (column_idx % 7) * 9;

        let mask = 0x1FFu64 << shift;

        let mut current = data[array_idx] as u64;
        current = (current & !mask) | ((val & 0x1FF) << shift);
        data[array_idx] = current as i64;
    }

    #[must_use]
    pub fn get(&self, heightmap: ChunkHeightmapType, x: i32, z: i32, min_y: i32) -> i32 {
        let data = match heightmap {
            ChunkHeightmapType::WorldSurface => &self.world_surface,
            ChunkHeightmapType::MotionBlocking => &self.motion_blocking,
            ChunkHeightmapType::MotionBlockingNoLeaves => &self.motion_blocking_no_leaves,
        };

        let Some(data) = data else {
            return min_y - 1;
        };

        let local_x = (x & 15) as usize;
        let local_z = (z & 15) as usize;
        let column_idx = local_z * 16 + local_x;

        let array_idx = column_idx / 7;
        let shift = (column_idx % 7) * 9;

        let current = data[array_idx] as u64;
        let val = (current >> shift) & 0x1FF;

        (val as i32) + min_y - 1
    }

    #[expect(clippy::too_many_arguments)]
    pub fn update<F>(
        &mut self,
        heightmap_type: ChunkHeightmapType,
        local_x: i32,
        local_y: i32,
        local_z: i32,
        block_state: &BlockState,
        min_y: i32,
        get_block: F,
    ) -> bool
    where
        F: Fn(i32) -> &'static BlockState,
    {
        let first_available = self.get(heightmap_type, local_x, local_z, min_y) + 1;
        if local_y <= first_available - 2 {
            return false;
        }

        if heightmap_type.is_opaque(block_state) {
            if local_y >= first_available {
                self.set(heightmap_type, local_x, local_z, local_y, min_y);
                return true;
            }
        } else if first_available - 1 == local_y {
            for y in (min_y..local_y).rev() {
                let state = get_block(y);
                if heightmap_type.is_opaque(state) {
                    self.set(heightmap_type, local_x, local_z, y, min_y);
                    return true;
                }
            }
            self.set(heightmap_type, local_x, local_z, min_y - 1, min_y);
            return true;
        }

        false
    }
}

/// The Heightmap for a completely empty chunk
impl Default for ChunkHeightmaps {
    fn default() -> Self {
        Self {
            motion_blocking: None,
            motion_blocking_no_leaves: None,
            world_surface: None,
        }
    }
}
