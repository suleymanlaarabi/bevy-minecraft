//! Chunk dimensions and conversions between world, chunk and cell coordinates.

use bevy::prelude::*;

/// Blocks per chunk side, fixed by Pumpkin's 16x16 columns.
pub(crate) const CHUNK_SIZE: i32 = 16;
/// Blocks per chunk column: Minecraft Y -64..320, stored from 0.
pub(crate) const CHUNK_HEIGHT: i32 = 384;
/// Height of one render section; a chunk column is meshed one section at a time.
pub(crate) const SECTION_HEIGHT: i32 = 16;
pub(crate) const SECTION_COUNT: usize = (CHUNK_HEIGHT / SECTION_HEIGHT) as usize;
pub(crate) const SECTION_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * SECTION_HEIGHT) as usize;
pub(crate) const CHUNK_VOLUME: usize = SECTION_VOLUME * SECTION_COUNT;
/// Every render section of a column, one bit each.
pub(crate) const ALL_SECTIONS: u32 = ((1u64 << SECTION_COUNT) - 1) as u32;
/// Light above the world and of untouched sky: sky 15, block 0.
pub(crate) const FULL_SKY: u8 = 0xf0;

const _: () = assert!(
    CHUNK_SIZE == 16 && SECTION_HEIGHT == 16,
    "cell indices pack x, z and y as 4-bit fields"
);
const _: () = assert!(
    CHUNK_HEIGHT % SECTION_HEIGHT == 0 && SECTION_COUNT <= 32,
    "sections must tile the column and fit the section mask"
);

/// Index of a local cell in Pumpkin's Y-major layout, `x + 16 * (z + 16 * y)`.
#[inline]
pub(crate) const fn cell_index(local: IVec3) -> Option<usize> {
    if (local.x as u32) < CHUNK_SIZE as u32
        && (local.z as u32) < CHUNK_SIZE as u32
        && (local.y as u32) < CHUNK_HEIGHT as u32
    {
        Some((local.x | local.z << 4 | local.y << 8) as usize)
    } else {
        None
    }
}

/// Inverse of [`cell_index`].
#[inline]
pub(crate) const fn cell_position(index: usize) -> IVec3 {
    IVec3::new(
        (index & 15) as i32,
        (index >> 8) as i32,
        (index >> 4 & 15) as i32,
    )
}

/// Chunk column containing a world cell, and the cell's position inside it.
#[inline]
pub(crate) const fn split_world_position(world: IVec3) -> (IVec2, IVec3) {
    (
        IVec2::new(
            world.x.div_euclid(CHUNK_SIZE),
            world.z.div_euclid(CHUNK_SIZE),
        ),
        IVec3::new(
            world.x.rem_euclid(CHUNK_SIZE),
            world.y,
            world.z.rem_euclid(CHUNK_SIZE),
        ),
    )
}

/// Chunk column containing a continuous world position.
#[inline]
pub fn chunk_at(world: Vec3) -> IVec2 {
    IVec2::new(
        (world.x.floor() as i32).div_euclid(CHUNK_SIZE),
        (world.z.floor() as i32).div_euclid(CHUNK_SIZE),
    )
}

/// The 3x3 slot of a neighbor offset, matching `ChunkNeighborhood`.
pub(crate) fn neighbor_slot(offset: IVec2) -> usize {
    ((offset.x + 1) + 3 * (offset.y + 1)) as usize
}
