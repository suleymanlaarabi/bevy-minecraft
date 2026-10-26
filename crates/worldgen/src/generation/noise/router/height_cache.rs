//! Bounded world-local cache of pure preliminary surface-height estimates.
//! Collisions replace slots; an in-flight Arc remains valid outside the map.
use std::sync::{Arc, Mutex, OnceLock};

const SHARDS: usize = 64;
const SLOTS_PER_SHARD: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct HeightCacheKey {
    pub column: u64,
    pub minimum_y: i32,
    pub maximum_y: i32,
    pub y_level_step_count: usize,
}

#[derive(Clone)]
struct Entry {
    key: HeightCacheKey,
    value: Arc<OnceLock<i32>>,
}

pub struct SharedHeightCache {
    shards: [Mutex<Box<[Option<Entry>]>>; SHARDS],
    enabled: bool,
    profile: bool,
}

impl Default for SharedHeightCache {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| {
                Mutex::new(vec![None; SLOTS_PER_SHARD].into_boxed_slice())
            }),
            enabled: std::env::var("WORLDGEN_HEIGHT_CACHE").as_deref() != Ok("off"),
            profile: std::env::var("WORLDGEN_PROFILE").as_deref() == Ok("1"),
        }
    }
}

impl SharedHeightCache {
    pub const CAPACITY: usize = SHARDS * SLOTS_PER_SHARD;

    pub fn get_or_compute(&self, key: HeightCacheKey, compute: impl FnOnce() -> i32) -> i32 {
        if !self.enabled {
            return compute();
        }
        let mut hash = key.column
            ^ (key.minimum_y as u64).rotate_left(17)
            ^ (key.maximum_y as u64).rotate_left(33)
            ^ (key.y_level_step_count as u64).rotate_left(49);
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        hash ^= hash >> 31;
        let index = hash as usize & (Self::CAPACITY - 1);
        let value = {
            let mut shard = self.shards[index % SHARDS]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let slot = &mut shard[index / SHARDS];
            if let Some(entry) = slot
                && entry.key == key
            {
                if self.profile {}
                entry.value.clone()
            } else {
                if self.profile {}
                let value = Arc::new(OnceLock::new());
                *slot = Some(Entry {
                    key,
                    value: value.clone(),
                });
                value
            }
        }; // Release the shard before computing or waiting on the cell.
        *value.get_or_init(compute)
    }
}
