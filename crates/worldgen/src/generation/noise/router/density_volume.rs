use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

pub use pumpkin_util::noise::volume::DensityVolume;

const BUFFER_SIZE_INCREMENT: usize = 16;
const MAX_REUSE_SIZE_FACTOR: usize = 2;
const MAX_POOLED_BYTES: usize = 16 * 1024 * 1024;
const MAX_POOLED_BUFFERS: usize = 128;

#[cfg(feature = "dev")]
fn memory_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("HOLLOW_DIAGNOSTICS").is_some()
            || std::env::var_os("HOLLOW_FLIGHT_BENCH").is_some()
    })
}

#[derive(Default)]
struct BufferPool {
    // At most 128 pointers: a sorted vector avoids allocating tree nodes and
    // per-size vectors every time a temporary is taken and returned.
    buffers: Vec<Box<[f32]>>,
    bytes: usize,
}
impl BufferPool {
    fn take(&mut self, min: usize, max: usize) -> Option<Box<[f32]>> {
        let index = self.buffers.partition_point(|buffer| buffer.len() < min);
        if self.buffers.get(index)?.len() > max {
            return None;
        }
        let buffer = self.buffers.remove(index);
        self.bytes -= size_of_val(buffer.as_ref());
        #[cfg(feature = "dev")]
        if memory_enabled() {
            record_buffer(1, size_of_val(buffer.as_ref()), false);
            POOLED_BYTES.fetch_sub(
                size_of_val(buffer.as_ref()),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        Some(buffer)
    }
    fn put(&mut self, value: Box<[f32]>) {
        let bytes = size_of_val(value.as_ref());
        if self.buffers.len() >= MAX_POOLED_BUFFERS || bytes > MAX_POOLED_BYTES - self.bytes {
            return;
        }
        let index = self
            .buffers
            .partition_point(|buffer| buffer.len() < value.len());
        self.buffers.insert(index, value);
        self.bytes += bytes;
        #[cfg(feature = "dev")]
        if memory_enabled() {
            record_buffer(1, bytes, true);
            POOLED_BYTES.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
#[cfg(feature = "dev")]
impl Drop for BufferPool {
    fn drop(&mut self) {
        if memory_enabled() {
            for buffer in &self.buffers {
                record_buffer(1, size_of_val(buffer.as_ref()), false);
            }
            POOLED_BYTES.fetch_sub(self.bytes, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
#[cfg(feature = "dev")]
static POOLED_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "dev")]
static ACTIVE_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "dev")]
static PEAK_ACTIVE_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "dev")]
static BUFFER_COUNTS: [[std::sync::atomic::AtomicUsize; 8]; 2] =
    [const { [const { std::sync::atomic::AtomicUsize::new(0) }; 8] }; 2];
#[cfg(feature = "dev")]
fn record_buffer(kind: usize, bytes: usize, add: bool) {
    // Capacity classes: <4 KiB, <16 KiB, <64 KiB, <256 KiB, <1 MiB,
    // <4 MiB, <16 MiB, and >=16 MiB. Counts distinguish reuse from active work.
    let limits = [
        4 << 10,
        16 << 10,
        64 << 10,
        256 << 10,
        1 << 20,
        4 << 20,
        16 << 20,
    ];
    let counter = &BUFFER_COUNTS[kind][limits.partition_point(|&limit| bytes >= limit)];
    if add {
        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else {
        counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}
#[cfg(feature = "dev")]
pub fn memory_buffer_counts() -> [[usize; 8]; 2] {
    BUFFER_COUNTS.each_ref().map(|counts| {
        counts
            .each_ref()
            .map(|value| value.load(std::sync::atomic::Ordering::Relaxed))
    })
}
#[cfg(feature = "dev")]
pub fn memory_bytes() -> [usize; 3] {
    use std::sync::atomic::Ordering::Relaxed;
    [
        ACTIVE_BYTES.load(Relaxed),
        POOLED_BYTES.load(Relaxed),
        PEAK_ACTIVE_BYTES.load(Relaxed),
    ]
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
        #[cfg(feature = "dev")]
        if memory_enabled() {
            use std::sync::atomic::Ordering::Relaxed;
            let bytes = size_of_val(values.as_ref());
            record_buffer(0, bytes, true);
            let active = ACTIVE_BYTES.fetch_add(bytes, Relaxed) + bytes;
            PEAK_ACTIVE_BYTES.fetch_max(active, Relaxed);
        }
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
        #[cfg(feature = "dev")]
        if memory_enabled() {
            record_buffer(0, size_of_val(values.as_ref()), false);
            ACTIVE_BYTES.fetch_sub(
                size_of_val(values.as_ref()),
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        DENSITY_BUFFER_POOL.with(|pool| {
            pool.borrow_mut().put(values);
        });
    }
}
