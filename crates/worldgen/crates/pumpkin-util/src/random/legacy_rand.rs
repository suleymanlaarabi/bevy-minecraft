use super::{
    RandomDeriver, RandomDeriverImpl, RandomGenerator, RandomImpl, gaussian::GaussianGenerator,
    hash_block_pos,
};

use crate::{math::java_string_hash, population_seed_fn};

/// Legacy random number generator compatible with older Minecraft versions.
///
/// This implementation uses a 48-bit linear congruential generator (LCG) with the formula:
/// `seed = (seed * 0x5DEECE66D + 11) & 0xFFFFFFFFFFFF`
///
/// It maintains compatibility with Minecraft's original random number generator used in
/// versions prior to 1.13. This is essential for world generation compatibility with
/// older worlds and for certain features that still rely on the legacy algorithm.
pub struct LegacyRand {
    /// The current 48-bit seed value (stored in lower 48 bits).
    seed: u64,
    /// Stored Gaussian value for the next Gaussian generation.
    internal_next_gaussian: Option<f64>,
}

impl LegacyRand {
    population_seed_fn!();

    /// Creates a new legacy random generator from the given seed.
    ///
    /// The seed is `XORed` with the constant `0x5DEECE66D` and masked to 48 bits.
    ///
    /// # Arguments
    /// - `seed` – The initial seed value.
    ///
    /// # Returns
    /// A new `LegacyRand` instance.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self {
            seed: (seed ^ 0x0005_DEEC_E66D) & 0xFFFF_FFFF_FFFF,
            internal_next_gaussian: None,
        }
    }

    /// Generates the next random value and advances the internal state.
    ///
    /// This implements the core LCG algorithm: `seed = seed * 0x5DEECE66D + 11`
    /// and returns the new seed.
    ///
    /// # Returns
    /// The new 48-bit seed value.
    const fn next_random(&mut self) -> i64 {
        let l = self.seed as i64;
        let m = l.wrapping_mul(0x0005_DEEC_E66D).wrapping_add(11) & 0xFFFFFFFFFFFF;
        self.seed = m as u64;
        m
    }

    /// Generates a random integer with the specified number of bits.
    ///
    /// # Arguments
    /// - `bits` – The number of bits to extract (1-48).
    ///
    /// # Returns
    /// A random value with the given number of bits.
    const fn next(&mut self, bits: u64) -> i32 {
        (self.next_random() >> (48 - bits)) as i32
    }
}

impl GaussianGenerator for LegacyRand {
    fn stored_next_gaussian(&self) -> Option<f64> {
        self.internal_next_gaussian
    }

    fn set_stored_next_gaussian(&mut self, value: Option<f64>) {
        self.internal_next_gaussian = value;
    }
}

impl RandomImpl for LegacyRand {
    fn skip(&mut self, count: i32) {
        // Advance exactly `count` single LCG steps using `next_i32()`.
        // `next_i64()` consumes two LCG steps, so we override the default.
        for _ in 0..count {
            self.next_i32();
        }
    }

    fn split(&mut self) -> Self {
        Self::from_seed(self.next_i64() as u64)
    }

    fn next_splitter(&mut self) -> RandomDeriver {
        RandomDeriver::Legacy(LegacySplitter::new(self.next_i64() as u64))
    }

    fn next_i32(&mut self) -> i32 {
        self.next(32)
    }

    fn next_bounded_i32(&mut self, bound: i32) -> i32 {
        if (bound & bound.wrapping_sub(1)) == 0 {
            (i64::from(bound).wrapping_mul(i64::from(self.next(31))) >> 31) as i32
        } else {
            loop {
                let i = self.next(31);
                let j = i % bound;
                if (i.wrapping_sub(j).wrapping_add(bound.wrapping_sub(1))) >= 0 {
                    return j;
                }
            }
        }
    }

    fn next_i64(&mut self) -> i64 {
        let i = self.next_i32();
        let j = self.next_i32();
        (i64::from(i) << 32).wrapping_add(i64::from(j))
    }

    fn next_bool(&mut self) -> bool {
        self.next(1) != 0
    }

    fn next_f32(&mut self) -> f32 {
        self.next(24) as f32 * 5.9604645E-8f32
    }

    fn next_f64(&mut self) -> f64 {
        let i = self.next(26);
        let j = self.next(27);
        let l = (i64::from(i) << 27).wrapping_add(i64::from(j));
        l as f64 * 1.1102230246251565e-16f64
    }

    fn next_gaussian(&mut self) -> f64 {
        self.calculate_gaussian()
    }
}

/// A splitter for creating derived legacy random generators.
///
/// This struct allows for deterministic creation of multiple independent
/// random generators from a single seed, which is essential for
/// parallelizable world generation.
#[derive(Clone)]
pub struct LegacySplitter {
    /// The base seed for deriving new generators.
    seed: u64,
}

impl LegacySplitter {
    /// Creates a new legacy splitter with the given seed.
    ///
    /// # Arguments
    /// - `seed` – The base seed value.
    ///
    /// # Returns
    /// A new `LegacySplitter` instance.
    const fn new(seed: u64) -> Self {
        Self { seed }
    }
}

impl RandomDeriverImpl for LegacySplitter {
    fn split_string(&self, seed: &str) -> RandomGenerator {
        let string_hash = java_string_hash(seed);

        RandomGenerator::Legacy(LegacyRand::from_seed((string_hash as u64) ^ self.seed))
    }

    fn split_u64(&self, seed: u64) -> RandomGenerator {
        RandomGenerator::Legacy(LegacyRand::from_seed(seed))
    }

    fn split_pos(&self, x: i32, y: i32, z: i32) -> RandomGenerator {
        let pos_hash = hash_block_pos(x, y, z);

        RandomGenerator::Legacy(LegacyRand::from_seed((pos_hash as u64) ^ self.seed))
    }
}

