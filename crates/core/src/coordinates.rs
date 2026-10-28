//! Chunk dimensions and conversions between world, chunk and cell coordinates.

use bevy_math::prelude::*;

/// Blocks per chunk side.
pub const CHUNK_SIZE: i32 = 16;
/// Height of one render section; a chunk column is meshed one section at a time.
pub const SECTION_HEIGHT: i32 = 16;
pub const SECTION_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * SECTION_HEIGHT) as usize;
/// Light above the world and of untouched sky: sky 15, block 0.
pub const FULL_SKY: u8 = 0xf0;

const _: () = assert!(
    CHUNK_SIZE == 16 && SECTION_HEIGHT == 16,
    "cell indices pack x, z and y as 4-bit fields"
);
/// Inverse of [`column_cell_index`].
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

/// Immutable geometry of one world. Cell and light arrays contain only active sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnLayout {
    min_y: i32,
    sections: u8,
}
impl Default for ColumnLayout {
    fn default() -> Self {
        Self::new(0, 24).expect("Default column layout")
    }
}
impl ColumnLayout {
    pub fn new(min_y: i32, sections: u8) -> Option<Self> {
        // Neighbor and lighting sampling require a cell of vertical address space on each side.
        (min_y > i32::MIN
            && sections > 0
            && sections <= 32
            && min_y
                .checked_add(i32::from(sections) * SECTION_HEIGHT)
                .is_some_and(|top| top < i32::MAX))
        .then_some(Self { min_y, sections })
    }
    pub const fn min_y(self) -> i32 {
        self.min_y
    }
    pub const fn height(self) -> i32 {
        self.sections as i32 * SECTION_HEIGHT
    }
    pub const fn max_y(self) -> i32 {
        self.min_y + self.height()
    }
    pub const fn section_count(self) -> usize {
        self.sections as usize
    }
    pub const fn mask(self) -> u32 {
        section_mask(self.section_count())
    }
    pub fn contains(self, y: i32) -> bool {
        (self.min_y..self.max_y()).contains(&y)
    }
    pub fn split(self, position: IVec3) -> (IVec2, IVec3) {
        let (chunk, mut local) = split_world_position(position);
        local.y = local.y.saturating_sub(self.min_y);
        (chunk, local)
    }
}
/// All valid section bits, including a full 32-section column without a 32-bit shift.
pub const fn section_mask(sections: usize) -> u32 {
    assert!(sections > 0 && sections <= 32, "Column section count");
    (u32::MAX as u64 >> (32 - sections)) as u32
}
/// Y-major local index, x + 16 * (z + 16 * y); y is relative to the column origin.
#[inline]
pub fn column_cell_index(local: IVec3, sections: usize) -> Option<usize> {
    ((local.x as u32) < 16 && (local.z as u32) < 16 && (local.y as u32) < sections as u32 * 16)
        .then(|| (local.x | local.z << 4 | local.y << 8) as usize)
}
