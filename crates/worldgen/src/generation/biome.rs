use pumpkin_util::math::{square_f64, vector3::Vector3};
use std::cell::RefCell;

use super::biome_coords;

// Reuse the seed-derived jitter of lattice corners, never the selected biome.
// The complete coordinate is checked after hashing, so collisions only cost work.
const JITTER_CACHE_SIZE: usize = 2048;
#[derive(Clone, Copy)]
struct JitterEntry {
    pos: [i32; 3],
    offset: [f64; 3],
}
struct BiomeJitterCache {
    seed: Option<i64>,
    entries: Box<[JitterEntry]>,
    cube_pos: [i32; 3],
    cube_offsets: [[f64; 3]; 8],
}
impl BiomeJitterCache {
    fn new() -> Self {
        Self {
            seed: None,
            cube_pos: [i32::MIN; 3],
            cube_offsets: [[0.0; 3]; 8],
            entries: vec![
                JitterEntry {
                    pos: [i32::MIN; 3],
                    offset: [0.0; 3]
                };
                JITTER_CACHE_SIZE
            ]
            .into_boxed_slice(),
        }
    }
    #[inline]
    fn prepare_cube(&mut self, seed: i64, pos: [i32; 3]) {
        if self.seed != Some(seed) || self.cube_pos != pos {
            let offsets = std::array::from_fn(|permutation| {
                self.offset(
                    seed,
                    pos[0] + ((permutation >> 2) & 1) as i32,
                    pos[1] + ((permutation >> 1) & 1) as i32,
                    pos[2] + (permutation & 1) as i32,
                )
            });
            self.cube_offsets = offsets;
            self.cube_pos = pos;
        }
    }
    #[inline]
    fn offset(&mut self, seed: i64, x: i32, y: i32, z: i32) -> [f64; 3] {
        if self.seed != Some(seed) {
            self.seed = Some(seed);
            for entry in &mut self.entries {
                entry.pos = [i32::MIN; 3];
            }
        }
        let hash = (x as u32).wrapping_mul(0x9e3779b1)
            ^ (y as u32).wrapping_mul(0x85ebca77)
            ^ (z as u32).wrapping_mul(0xc2b2ae3d);
        let entry = &mut self.entries[hash as usize & (JITTER_CACHE_SIZE - 1)];
        if entry.pos != [x, y, z] {
            entry.pos = [x, y, z];
            entry.offset = jitter(seed, x, y, z);
        }
        entry.offset
    }
}
thread_local! {
    static BIOME_JITTER: RefCell<BiomeJitterCache> = RefCell::new(BiomeJitterCache::new());
}

// This blends biome boundaries, returning the exact original winning corner.
pub fn get_biome_blend(
    bottom_y: i8,
    height: u16,
    seed: i64,
    x: i32,
    y: i32,
    z: i32,
) -> Vector3<i32> {
    BIOME_JITTER.with_borrow_mut(|cache| {
        cache.prepare_cube(seed, [(x - 2) >> 2, (y - 2) >> 2, (z - 2) >> 2]);
        let mut permutation = 0;
        blend(bottom_y, height, x, y, z, |_bx, _by, _bz, px, py, pz| {
            let [ox, oy, oz] = cache.cube_offsets[permutation];
            permutation += 1;
            square_f64(pz + oz) + square_f64(py + oy) + square_f64(px + ox)
        })
    })
}

/// Scalar reference used to validate the bounded per-worker jitter cache.
pub fn get_biome_blend_uncached(
    bottom_y: i8,
    height: u16,
    seed: i64,
    x: i32,
    y: i32,
    z: i32,
) -> Vector3<i32> {
    blend(bottom_y, height, x, y, z, |bx, by, bz, px, py, pz| {
        score_permutation(seed, bx, by, bz, px, py, pz)
    })
}

#[inline]
fn blend(
    bottom_y: i8,
    height: u16,
    x: i32,
    y: i32,
    z: i32,
    mut score: impl FnMut(i32, i32, i32, f64, f64, f64) -> f64,
) -> Vector3<i32> {
    // This is the "left" side of the biome boundary
    let offset_x = x - 2;
    let offset_y = y - 2;
    let offset_z = z - 2;
    let biome_x = biome_coords::from_block(offset_x);
    let biome_y = biome_coords::from_block(offset_y);
    let biome_z = biome_coords::from_block(offset_z);
    // &'ing 3 gives values of 0-3, it is also the data we removed when converting to biome coords
    // This is effectively "quarters" into the biome
    // Original was "/ 4.0" but we use "* 0.25" as multiplication can be faster
    let quarters_x = (offset_x & 0b11) as f64 * 0.25;
    let quarters_y = (offset_y & 0b11) as f64 * 0.25;
    let quarters_z = (offset_z & 0b11) as f64 * 0.25;

    let mut best_permutation = 0;
    let mut best_score = f64::INFINITY;
    for permutation in 0..8 {
        let should_maintain_x = (permutation & 0b100) == 0;
        let should_maintain_y = (permutation & 0b010) == 0;
        let should_maintain_z = (permutation & 0b001) == 0;

        // If we are shifting, add 1 to the biome coords
        let shifted_biome_x = if should_maintain_x {
            biome_x
        } else {
            biome_x + 1
        };
        let shifted_biome_y = if should_maintain_y {
            biome_y
        } else {
            biome_y + 1
        };
        let shifted_biome_z = if should_maintain_z {
            biome_z
        } else {
            biome_z + 1
        };

        // And reflect the "quarters" across the shift
        let shifted_quarters_x = if should_maintain_x {
            quarters_x
        } else {
            quarters_x - 1.0
        };
        let shifted_quarters_y = if should_maintain_y {
            quarters_y
        } else {
            quarters_y - 1.0
        };
        let shifted_quarters_z = if should_maintain_z {
            quarters_z
        } else {
            quarters_z - 1.0
        };

        let permutation_score = score(
            shifted_biome_x,
            shifted_biome_y,
            shifted_biome_z,
            shifted_quarters_x,
            shifted_quarters_y,
            shifted_quarters_z,
        );

        if best_score > permutation_score {
            best_score = permutation_score;
            best_permutation = permutation;
        }
    }

    // Now check if we want to use the "left" side or the "right" side
    let biome_x = if (best_permutation & 0b100) == 0 {
        biome_x
    } else {
        biome_x + 1
    };
    let biome_y = if (best_permutation & 0b010) == 0 {
        biome_y
    } else {
        biome_y + 1
    };
    let biome_z = if (best_permutation & 0b001) == 0 {
        biome_z
    } else {
        biome_z + 1
    };

    // Java's `getBiomeForNoiseGen`
    let bottom_y = bottom_y as i32;
    let biome_bottom = biome_coords::from_block(bottom_y);
    let biome_top = biome_bottom + biome_coords::from_block(height as i32) - 1;
    let biome_y = biome_y.clamp(biome_bottom, biome_top);

    Vector3::new(biome_x, biome_y, biome_z)
}

// This is effectively getting a random offset (+/- 0.0-0.8ish) to our biome position quarters and
// returning a hypotenuse squared of the parts + the offset
const fn score_permutation(
    seed: i64,
    x: i32,
    y: i32,
    z: i32,
    x_part: f64,
    y_part: f64,
    z_part: f64,
) -> f64 {
    let [offset_x, offset_y, offset_z] = jitter(seed, x, y, z);
    square_f64(z_part + offset_z) + square_f64(y_part + offset_y) + square_f64(x_part + offset_x)
}

#[inline]
const fn jitter(seed: i64, x: i32, y: i32, z: i32) -> [f64; 3] {
    let mix = salt_mix(seed, x as i64);
    let mix = salt_mix(mix, y as i64);
    let mix = salt_mix(mix, z as i64);
    let mix = salt_mix(mix, x as i64);
    let mix = salt_mix(mix, y as i64);
    let mix = salt_mix(mix, z as i64);
    let offset_x = scale_mix(mix);
    let mix = salt_mix(mix, seed);
    let offset_y = scale_mix(mix);
    let mix = salt_mix(mix, seed);
    let offset_z = scale_mix(mix);
    [offset_x, offset_y, offset_z]
}

#[inline]
pub const fn scale_mix(l: i64) -> f64 {
    // Shifting and then masking with 1023 (1024 - 1)
    // This is mathematically identical to floor_mod(l >> 24, 1024)
    // but executes in a single CPU cycle.
    let d = ((l >> 24) & 1023) as f64 / 1024.0;

    (d - 0.5) * 0.9
}

#[inline]
const fn salt_mix(seed: i64, salt: i64) -> i64 {
    let mixed_seed = seed.wrapping_mul(
        seed.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407),
    );
    mixed_seed.wrapping_add(salt)
}

