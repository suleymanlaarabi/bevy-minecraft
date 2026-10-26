use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

pub use pumpkin_util::noise::volume::DensityVolume;

const BUFFER_SIZE_INCREMENT: usize = 16;
const MAX_REUSE_SIZE_FACTOR: usize = 2;
const MAX_POOLED_BYTES: usize = 16 * 1024 * 1024;
const MAX_POOLED_BUFFERS: usize = 128;

#[derive(Default)]
struct BufferPool {
    buckets: BTreeMap<usize, Vec<Box<[f32]>>>,
    bytes: usize,
    count: usize,
}

impl BufferPool {
    fn take(&mut self, min: usize, max: usize) -> Option<Box<[f32]>> {
        let size = *self.buckets.range(min..=max).next()?.0;
        let bucket = self.buckets.get_mut(&size)?;
        let value = bucket.pop()?;
        if bucket.is_empty() {
            self.buckets.remove(&size);
        }
        self.bytes -= size * size_of::<f32>();
        self.count -= 1;
        Some(value)
    }

    fn put(&mut self, value: Box<[f32]>) {
        let bytes = size_of_val(value.as_ref());
        if self.count >= MAX_POOLED_BUFFERS || bytes > MAX_POOLED_BYTES - self.bytes {
            return;
        }
        self.buckets.entry(value.len()).or_default().push(value);
        self.count += 1;
        self.bytes += bytes;
    }
}

thread_local! {
    static DENSITY_BUFFER_POOL: RefCell<BufferPool> = RefCell::new(BufferPool::default());
}

pub struct DensityBuffer {
    values: Box<[f32]>,
    len: usize,
}

impl DensityBuffer {
    #[must_use]
    pub fn acquire(volume: &DensityVolume) -> Self {
        Self::with_len(volume.size())
    }

    #[must_use]
    pub fn with_len(len: usize) -> Self {
        let min_capacity = len.next_multiple_of(BUFFER_SIZE_INCREMENT);
        let values = DENSITY_BUFFER_POOL
            .with(|pool| {
                pool.borrow_mut()
                    .take(min_capacity, min_capacity * MAX_REUSE_SIZE_FACTOR)
            })
            .unwrap_or_else(|| vec![0.0; min_capacity].into_boxed_slice());
        Self { values, len }
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.values.len()
    }
}

impl Deref for DensityBuffer {
    type Target = [f32];

    #[inline]
    fn deref(&self) -> &[f32] {
        &self.values[..self.len]
    }
}

impl DerefMut for DensityBuffer {
    #[inline]
    fn deref_mut(&mut self) -> &mut [f32] {
        &mut self.values[..self.len]
    }
}

impl Drop for DensityBuffer {
    fn drop(&mut self) {
        let values = std::mem::take(&mut self.values);
        DENSITY_BUFFER_POOL.with(|pool| {
            pool.borrow_mut().put(values);
        });
    }
}
