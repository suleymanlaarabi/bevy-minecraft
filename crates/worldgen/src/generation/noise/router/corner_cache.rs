//! Bounded world-local cache of interpolation corner columns.
//!
//! A chunk's coarse corner grid has 3x3 columns, of which only 2x2 are its own:
//! the others lie on borders shared with its neighbors. Distance-ordered
//! generation makes those neighbors recent, so publishing each column once
//! avoids recomputing every interpolated noise on shared borders.
//! Collisions replace slots; published values are immutable.
use std::sync::{Arc, Mutex};

const SHARDS: usize = 64;
const SLOTS_PER_SHARD: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ColumnKey {
    pub x: i32,
    pub z: i32,
    pub min_y: i32,
    pub size_y: u32,
}

type Slot = Option<(ColumnKey, Arc<[f32]>)>;

pub struct CornerColumnCache {
    shards: [Mutex<Box<[Slot]>>; SHARDS],
}

impl Default for CornerColumnCache {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| {
                Mutex::new(vec![None; SLOTS_PER_SHARD].into_boxed_slice())
            }),
        }
    }
}

impl CornerColumnCache {
    const CAPACITY: usize = SHARDS * SLOTS_PER_SHARD;

    fn slot(key: ColumnKey) -> usize {
        let mut hash = (key.x as u32 as u64) | ((key.z as u32 as u64) << 32);
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        (hash ^ (hash >> 31)) as usize & (Self::CAPACITY - 1)
    }

    pub fn get(&self, key: ColumnKey) -> Option<Arc<[f32]>> {
        let index = Self::slot(key);
        let shard = self.shards[index % SHARDS]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &shard[index / SHARDS] {
            Some((cached, values)) if *cached == key => Some(values.clone()),
            _ => None,
        }
    }

    pub fn insert(&self, key: ColumnKey, values: Arc<[f32]>) {
        let index = Self::slot(key);
        let mut shard = self.shards[index % SHARDS]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        shard[index / SHARDS] = Some((key, values));
    }
}
