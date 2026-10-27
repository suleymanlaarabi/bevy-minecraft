//! Chunk dimensions and conversions between world, chunk and cell coordinates.

use bevy_math::prelude::*;

/// Blocks per chunk side.
pub const CHUNK_SIZE: i32 = 16;
/// Default column height, stored from zero.
pub const CHUNK_HEIGHT: i32 = 384;
/// Height of one render section; a chunk column is meshed one section at a time.
pub const SECTION_HEIGHT: i32 = 16;
pub const SECTION_COUNT: usize = (CHUNK_HEIGHT / SECTION_HEIGHT) as usize;
pub const SECTION_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * SECTION_HEIGHT) as usize;
pub const CHUNK_VOLUME: usize = SECTION_VOLUME * SECTION_COUNT;
/// Every render section of a column, one bit each.
pub const ALL_SECTIONS: u32 = ((1u64 << SECTION_COUNT) - 1) as u32;
/// Light above the world and of untouched sky: sky 15, block 0.
pub const FULL_SKY: u8 = 0xf0;

const _: () = assert!(
    CHUNK_SIZE == 16 && SECTION_HEIGHT == 16,
    "cell indices pack x, z and y as 4-bit fields"
);
const _: () = assert!(
    CHUNK_HEIGHT % SECTION_HEIGHT == 0 && SECTION_COUNT <= 32,
    "sections must tile the column and fit the section mask"
);

/// Index of a local cell in the Y-major layout, `x + 16 * (z + 16 * y)`.
#[inline]
pub const fn cell_index(local: IVec3) -> Option<usize> {
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
pub const fn cell_position(index: usize) -> IVec3 {
    IVec3::new(
        (index & 15) as i32,
        (index >> 8) as i32,
        (index >> 4 & 15) as i32,
    )
}

/// Chunk column containing a world cell, and the cell's position inside it.
#[inline]
pub const fn split_world_position(world: IVec3) -> (IVec2, IVec3) {
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
pub fn neighbor_slot(offset: IVec2) -> usize {
    ((offset.x + 1) + 3 * (offset.y + 1)) as usize
}

/// Saturating priority distance; teleports cannot overflow chunk scheduling keys.
pub fn chunk_distance_squared(a: IVec2, b: IVec2) -> i32 {
    let x = (i64::from(a.x) - i64::from(b.x)).unsigned_abs();
    let z = (i64::from(a.y) - i64::from(b.y)).unsigned_abs();
    x.saturating_mul(x)
        .saturating_add(z.saturating_mul(z))
        .min(i32::MAX as u64) as i32
}
