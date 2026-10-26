use crate::population_seed_fn;

use super::{
    RandomDeriver, RandomDeriverImpl, RandomGenerator, RandomImpl, gaussian::GaussianGenerator,
    hash_block_pos,
};

/// Xoroshiro128+ random number generator implementation.
///
/// This is a modern, fast random number generator used in newer Minecraft versions.
/// It implements the Xoroshiro128+ algorithm, which has better statistical properties
/// and performance than the legacy LCG implementation.
///
/// The generator maintains 128 bits of state split into two 64-bit words (`lo` and `hi`)
/// and uses XOR, rotation, and shift operations to produce high-quality random numbers.
pub struct Xoroshiro {
    /// Lower 64 bits of the generator state.
    lo: u64,
    /// Higher 64 bits of the generator state.
    hi: u64,
    /// Stored Gaussian value for the next Gaussian generation.
    internal_next_gaussian: Option<f64>,
}

impl Xoroshiro {
    population_seed_fn!();

    /// Creates a new Xoroshiro generator from the given seed.
    ///
    /// The seed is mixed using the Stafford 13 mixing function to ensure
    /// good distribution of the initial state.
    ///
    /// # Arguments
    /// - `seed` – The initial seed value.
    ///
    /// # Returns
    /// A new `Xoroshiro` instance.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        let (lo, hi) = Self::mix_u64(seed);
        let lo = mix_stafford_13(lo);
        let hi = mix_stafford_13(hi);
        Self::new(lo, hi)
    }

    /// Creates a new Xoroshiro generator with the given state words.
    ///
    /// If both state words are zero, they are replaced with default values
    /// to avoid the all-zero state which is invalid for the algorithm.
    ///
    /// # Arguments
    /// - `lo` – The lower 64-bit state word.
    /// - `hi` – The higher 64-bit state word.
    ///
    /// # Returns
    /// A new `Xoroshiro` instance.
    const fn new(lo: u64, hi: u64) -> Self {
        let (lo, hi) = if (lo | hi) == 0 {
            (0x9E3779B97F4A7C15, 0x6A09E667F3BCC909)
        } else {
            (lo, hi)
        };
        Self {
            lo,
            hi,
            internal_next_gaussian: None,
        }
    }

    /// Mixes a 64-bit seed into two 64-bit state words.
    ///
    /// This uses the golden ratio constants to produce two distinct
    /// starting values from a single seed.
    ///
    /// # Arguments
    /// - `seed` – The seed value.
    ///
    /// # Returns
    /// A tuple of two 64-bit values for initializing the state.
    const fn mix_u64(seed: u64) -> (u64, u64) {
        let l = seed ^ 0x6A09E667F3BCC909;
        let m = l.wrapping_add(0x9E3779B97F4A7C15);
        (l, m)
    }

    /// Creates a new Xoroshiro generator without mixing the seed.
    ///
    /// This is used for testing and when the seed is already properly mixed.
    ///
    /// # Arguments
    /// - `seed` – The seed value (will be mixed using the standard mix function).
    ///
    /// # Returns
    /// A new `Xoroshiro` instance.
    #[must_use]
    pub const fn from_seed_unmixed(seed: u64) -> Self {
        let (lo, hi) = Self::mix_u64(seed);
        Self::new(lo, hi)
    }

    /// Generates a random value with the specified number of bits.
    ///
    /// # Arguments
    /// - `bits` – The number of bits to extract (0-64).
    ///
    /// # Returns
    /// A random value with the given number of bits.
    pub(super) const fn next(&mut self, bits: u64) -> u64 {
        self.next_random() >> (64 - bits)
    }

    /// Generates the next random value and advances the generator state.
    ///
    /// This implements the core Xoroshiro128+ algorithm:
    /// `result = state.lo + state.hi`
    /// Then updates the state using XOR, rotation, and shift operations.
    ///
    /// # Returns
    /// A 64-bit random value.
    const fn next_random(&mut self) -> u64 {
        let l = self.lo;
        let m = self.hi;
        let n = l.wrapping_add(m).rotate_left(17).wrapping_add(l);
        let m = m ^ l;
        self.lo = l.rotate_left(49) ^ m ^ (m << 21);
        self.hi = m.rotate_left(28);
        n
    }

    pub const fn next_splitter(&mut self) -> XoroshiroSplitter {
        XoroshiroSplitter {
            lo: self.next_random(),
            hi: self.next_random(),
        }
    }
}

impl GaussianGenerator for Xoroshiro {
    fn stored_next_gaussian(&self) -> Option<f64> {
        self.internal_next_gaussian
    }

    fn set_stored_next_gaussian(&mut self, value: Option<f64>) {
        self.internal_next_gaussian = value;
    }
}

/// Stafford 13 mixing function for seed initialization.
///
/// This is a high-quality integer hash function used to mix the initial seed
/// into the generator state for better distribution.
///
/// # Arguments
/// - `z` – The value to mix.
///
/// # Returns
/// The mixed value.
const fn mix_stafford_13(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

impl RandomImpl for Xoroshiro {
    fn split(&mut self) -> Self {
        Self::new(self.next_random(), self.next_random())
    }

    fn next_splitter(&mut self) -> RandomDeriver {
        RandomDeriver::Xoroshiro(self.next_splitter())
    }

    fn next_i32(&mut self) -> i32 {
        self.next_random() as i32
    }

    fn next_bounded_i32(&mut self, bound: i32) -> i32 {
        let mut l = (self.next_i32() as u64) & 0xFFFF_FFFF;
        let mut m = l.wrapping_mul(bound as u64);
        let mut n = m & 0xFFFF_FFFF;
        if n < bound as u64 {
            let i = ((!bound).wrapping_add(1) as u64) % (bound as u64);
            while n < i {
                l = (self.next_i32() as u64) & 0xFFFF_FFFF;
                m = l.wrapping_mul(bound as u64);
                n = m & 0xFFFF_FFFF;
            }
        }
        let o = m >> 32;
        o as i32
    }

    fn next_i64(&mut self) -> i64 {
        self.next_random() as i64
    }

    fn next_bool(&mut self) -> bool {
        (self.next_random() & 1) != 0
    }

    fn next_f32(&mut self) -> f32 {
        self.next(24) as f32 * 5.960_464_5E-8f32
    }

    fn next_f64(&mut self) -> f64 {
        self.next(53) as f64 * f64::from(1.110_223E-16f32)
    }

    fn next_gaussian(&mut self) -> f64 {
        self.calculate_gaussian()
    }
}

/// A splitter for creating derived Xoroshiro random generators.
///
/// This struct allows for deterministic creation of multiple independent
/// random generators from a single seed, which is essential for
/// parallelizable world generation.
pub struct XoroshiroSplitter {
    /// Lower 64 bits of the splitter state.
    lo: u64,
    /// Higher 64 bits of the splitter state.
    hi: u64,
}

impl XoroshiroSplitter {
    #[must_use]
    pub fn split_string(&self, seed: &str) -> Xoroshiro {
        let bytes = <md5::Md5 as md5::Digest>::digest(seed.as_bytes());
        let l = u64::from_be_bytes(bytes[0..8].try_into().unwrap_or([0; 8]));
        let m = u64::from_be_bytes(bytes[8..16].try_into().unwrap_or([0; 8]));

        Xoroshiro::new(l ^ self.lo, m ^ self.hi)
    }

    #[must_use]
    pub const fn from_lo_and_hi(&self, lo: u64, hi: u64) -> Xoroshiro {
        Xoroshiro::new(lo ^ self.lo, hi ^ self.hi)
    }

    #[must_use]
    pub const fn split_pos(&self, x: i32, y: i32, z: i32) -> Xoroshiro {
        let l = hash_block_pos(x, y, z) as u64;
        let m = l ^ self.lo;

        Xoroshiro::new(m, self.hi)
    }
}

impl RandomDeriverImpl for XoroshiroSplitter {
    fn split_string(&self, seed: &str) -> RandomGenerator {
        RandomGenerator::Xoroshiro(self.split_string(seed))
    }

    fn split_u64(&self, seed: u64) -> RandomGenerator {
        RandomGenerator::Xoroshiro(Xoroshiro::new(seed ^ self.lo, seed ^ self.hi))
    }

    fn split_pos(&self, x: i32, y: i32, z: i32) -> RandomGenerator {
        RandomGenerator::Xoroshiro(self.split_pos(x, y, z))
    }
}

