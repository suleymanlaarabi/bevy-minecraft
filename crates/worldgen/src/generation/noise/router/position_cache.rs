//! Bounded, world-owned memoization of pure position queries.
//! A displaced entry stays alive for its in-flight users; locks never cover work.
//! Slots are two-way sets ordered by recency: neighboring chunks revisit the same
//! positions, and a direct-mapped pair of colliding keys would evict each other.
use std::{
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

const SHARDS: usize = 64;
const SLOTS: usize = 128;

struct Entry<K, V> {
    key: K,
    value: Arc<OnceLock<V>>,
}

pub struct PositionCache<K, V> {
    shards: [Mutex<Box<[Option<Entry<K, V>>]>>; SHARDS],
}

impl<K, V> Default for PositionCache<K, V> {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Mutex::new((0..SLOTS).map(|_| None).collect())),
        }
    }
}

impl<K: Hash + Eq + Copy, V: Copy> PositionCache<K, V> {
    pub fn get_or_compute(&self, key: K, compute: impl FnOnce() -> V) -> V {
        let mut hash = rustc_hash::FxHasher::default();
        key.hash(&mut hash);
        let mut hash = hash.finish();
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d049bb133111eb);
        let index = (hash ^ (hash >> 31)) as usize;
        let value = {
            let mut shard = self.shards[index & (SHARDS - 1)]
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let first = (index / SHARDS) & (SLOTS - 2);
            let hit =
                |slot: &Option<Entry<K, V>>| slot.as_ref().is_some_and(|entry| entry.key == key);
            if !hit(&shard[first]) {
                // Promote the second way, or demote the first one before replacing it.
                shard.swap(first, first + 1);
                if !hit(&shard[first]) {
                    shard[first] = Some(Entry {
                        key,
                        value: Arc::new(OnceLock::new()),
                    });
                }
            }
            shard[first].as_ref().unwrap().value.clone()
        };
        *value.get_or_init(compute)
    }
}
