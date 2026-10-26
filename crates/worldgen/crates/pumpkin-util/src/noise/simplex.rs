use std::hash::Hash;

use num_traits::Pow;

use crate::random::{RandomImpl, legacy_rand::LegacyRand};

use super::GRADIENTS;

/// A 3D Simplex noise sampler implementation.
///
/// Simplex noise is an improved gradient noise function developed by Ken Perlin as an
/// alternative to classic Perlin noise. It has several advantages:
/// - Lower computational complexity (O(n²) vs O(2ⁿ)).
/// - No directional artifacts (better isotropy).
/// - Well-defined and continuous gradients.
/// - Easier to implement for higher dimensions.
///
/// This implementation provides both 2D and 3D simplex noise sampling.
#[derive(Clone)]
pub struct SimplexNoiseSampler {
    /// Permutation table for hashing coordinates (size 256, duplicated for easy wrapping).
    permutation: [u8; 256],
    /// X-coordinate origin offset (randomized to avoid symmetry).
    x_origin: f64,
    /// Y-coordinate origin offset (randomized to avoid symmetry).
    y_origin: f64,
    /// Z-coordinate origin offset (randomized to avoid symmetry).
    z_origin: f64,
}

impl PartialEq for SimplexNoiseSampler {
    fn eq(&self, other: &Self) -> bool {
        self.permutation == other.permutation
            && self.x_origin.to_le_bytes() == other.x_origin.to_le_bytes()
            && self.y_origin.to_le_bytes() == other.y_origin.to_le_bytes()
            && self.z_origin.to_le_bytes() == other.z_origin.to_le_bytes()
    }
}

impl Eq for SimplexNoiseSampler {}

impl Hash for SimplexNoiseSampler {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.permutation.hash(state);
        self.x_origin.to_le_bytes().hash(state);
        self.y_origin.to_le_bytes().hash(state);
        self.z_origin.to_le_bytes().hash(state);
    }
}

impl SimplexNoiseSampler {
    /// √3 constant used for skewing and unskewing factors.
    const SQRT_3: f64 = 1.7320508075688772f64;
    /// Skew factor for 2D simplex transformation (maps square grid to simplex grid).
    const SKEW_FACTOR_2D: f64 = 0.5f64 * (Self::SQRT_3 - 1f64);
    /// Unskew factor for 2D simplex transformation (maps simplex grid back to square grid).
    const UNSKEW_FACTOR_2D: f64 = (3f64 - Self::SQRT_3) / 6f64;

    /// Creates a new Simplex noise sampler with randomized origin and permutation table.
    ///
    /// # Arguments
    /// - `random` – The random number generator to use for initialization.
    ///
    /// # Returns
    /// A new `SimplexNoiseSampler` instance.
    pub fn new(random: &mut impl RandomImpl) -> Self {
        let x_origin = random.next_f64() * 256f64;
        let y_origin = random.next_f64() * 256f64;
        let z_origin = random.next_f64() * 256f64;

        let mut permutation = [0u8; 256];

        permutation
            .iter_mut()
            .enumerate()
            .for_each(|(i, x)| *x = i as u8);

        for i in 0..256 {
            let j = random.next_bounded_i32(256 - i) as usize;
            permutation.swap(i as usize, i as usize + j);
        }

        Self {
            permutation,
            x_origin,
            y_origin,
            z_origin,
        }
    }

    /// Maps an integer coordinate through the permutation table.
    ///
    /// # Arguments
    /// - `input` – The input coordinate.
    ///
    /// # Returns
    /// A hashed value in the range [0, 255].
    #[inline]
    fn map(&self, input: i32) -> i32 {
        i32::from(self.permutation[(input & 0xFF) as usize])
    }

    /// Computes the contribution of a single simplex corner.
    ///
    /// This function applies the simplex noise falloff kernel: (r² - |dist|²)⁴ · gradient.
    ///
    /// # Arguments
    /// - `gradient_index` – Index into the gradients array for this corner.
    /// - `x` – The X offset from the corner.
    /// - `y` – The Y offset from the corner.
    /// - `z` – The Z offset from the corner.
    /// - `distance` – The kernel radius (typically 0.5 for 2D, 0.6 for 3D).
    ///
    /// # Returns
    /// The contribution of this corner to the final noise value.
    const fn grad(gradient_index: usize, x: f64, y: f64, z: f64, distance: f64) -> f64 {
        let d = distance - x * x - y * y - z * z;
        if d < 0f64 {
            0f64
        } else {
            let d = d * d;
            d * d * GRADIENTS[gradient_index].dot(x, y, z)
        }
    }

    /// Samples 2D Simplex noise at the given coordinates.
    ///
    /// The noise value is in the approximate range [-1, 1].
    ///
    /// # Arguments
    /// - `x` – The X coordinate.
    /// - `y` – The Y coordinate.
    ///
    /// # Returns
    /// A noise value in the range [-1, 1].
    #[must_use]
    #[expect(clippy::many_single_char_names)]
    #[expect(clippy::suboptimal_flops)]
    pub fn sample_2d(&self, x: f64, y: f64) -> f64 {
        let d = (x + y) * Self::SKEW_FACTOR_2D;
        let i = (x + d).floor() as i32;
        let j = (y + d).floor() as i32;

        let e = f64::from(i.wrapping_add(j)) * Self::UNSKEW_FACTOR_2D;
        let f = f64::from(i) - e;
        let g = f64::from(j) - e;

        let h = x - f;
        let k = y - g;

        let (l, m) = if h > k { (1, 0) } else { (0, 1) };

        let n = h - f64::from(l) + Self::UNSKEW_FACTOR_2D;
        let o = k - f64::from(m) + Self::UNSKEW_FACTOR_2D;
        let p = 2.0 * Self::UNSKEW_FACTOR_2D + (h - 1.0);
        let q = 2.0 * Self::UNSKEW_FACTOR_2D + (k - 1.0);

        let r = i & 0xFF;
        let s = j & 0xFF;

        let t = self.map(r.wrapping_add(self.map(s))) % 12;
        let u = self.map(r.wrapping_add(l).wrapping_add(self.map(s.wrapping_add(m)))) % 12;
        let v = self.map(r.wrapping_add(1).wrapping_add(self.map(s.wrapping_add(1)))) % 12;

        let w = Self::grad(t as usize, h, k, 0.0, 0.5);
        let z = Self::grad(u as usize, n, o, 0.0, 0.5);
        let aa = Self::grad(v as usize, p, q, 0.0, 0.5);

        70.0 * (w + z + aa)
    }

    /// Samples 3D Simplex noise at the given coordinates.
    ///
    /// The noise value is in the approximate range [-1, 1].
    ///
    /// # Arguments
    /// - `x` – The X coordinate.
    /// - `y` – The Y coordinate.
    /// - `z` – The Z coordinate.
    ///
    /// # Returns
    /// A noise value in the range [-1, 1].
    #[must_use]
    #[expect(clippy::many_single_char_names)]
    pub fn sample_3d(&self, x: f64, y: f64, z: f64) -> f64 {
        let e = (x + y + z) * 0.3333333333333333;

        let i = (x + e).floor() as i32;
        let j = (y + e).floor() as i32;
        let k = (z + e).floor() as i32;

        let g = f64::from(i.wrapping_add(j).wrapping_add(k)) * 0.16666666666666666;
        let h = f64::from(i) - g;
        let l = f64::from(j) - g;
        let m = f64::from(k) - g;

        let n = x - h;
        let o = y - l;
        let p = z - m;

        let (q, r, s, t, u, v) = if n >= o {
            if o >= p {
                (1, 0, 0, 1, 1, 0)
            } else if n >= p {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if o < p {
            (0, 0, 1, 0, 1, 1)
        } else if n < p {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };

        let w = n - f64::from(q) + 0.16666666666666666;
        let aa = o - f64::from(r) + 0.16666666666666666;
        let ab = p - f64::from(s) + 0.16666666666666666;

        let ac = n - f64::from(t) + 0.3333333333333333;
        let ad = o - f64::from(u) + 0.3333333333333333;
        let ae = p - f64::from(v) + 0.3333333333333333;

        let af = n - 1.0 + 0.5;
        let ag = o - 1.0 + 0.5;
        let ah = p - 1.0 + 0.5;

        let ai = i & 0xFF;
        let aj = j & 0xFF;
        let ak = k & 0xFF;

        let al = self.map(ai.wrapping_add(self.map(aj.wrapping_add(self.map(ak))))) % 12;
        //TODO: extract duplication to a helper method.
        let am = self.map(
            ai.wrapping_add(q).wrapping_add(
                self.map(
                    aj.wrapping_add(r)
                        .wrapping_add(self.map(ak.wrapping_add(s))),
                ),
            ),
        ) % 12;
        let an = self.map(
            ai.wrapping_add(t).wrapping_add(
                self.map(
                    aj.wrapping_add(u)
                        .wrapping_add(self.map(ak.wrapping_add(v))),
                ),
            ),
        ) % 12;
        let ao = self.map(
            ai.wrapping_add(1).wrapping_add(
                self.map(
                    aj.wrapping_add(1)
                        .wrapping_add(self.map(ak.wrapping_add(1))),
                ),
            ),
        ) % 12;

        let ap = Self::grad(al as usize, n, o, p, 0.6);
        let aq = Self::grad(am as usize, w, aa, ab, 0.6);
        let ar = Self::grad(an as usize, ac, ad, ae, 0.6);
        let az = Self::grad(ao as usize, af, ag, ah, 0.6);

        32.0 * (ap + aq + ar + az)
    }
}

/// A multi-octave Simplex noise sampler that combines multiple noise layers.
///
/// Octave noise, also known as fractal noise, combines multiple octaves of Simplex noise
/// with different frequencies and amplitudes to create more complex, natural-looking patterns.
/// Each octave adds finer detail to the overall noise.
pub struct OctaveSimplexNoiseSampler {
    /// The list of samplers for each octave (None for unused octaves).
    octave_samplers: Vec<Option<SimplexNoiseSampler>>,
    /// The persistence factor (amplitude scaling between octaves).
    persistence: f64,
    /// The lacunarity factor (frequency scaling between octaves).
    lacunarity: f64,
}

impl OctaveSimplexNoiseSampler {
    /// Creates a new octave Simplex noise sampler.
    ///
    /// This constructor follows Minecraft's legacy initialization pattern, using a combination
    /// of Xoroshiro and `LegacyRand` to ensure compatibility with existing world generation.
    ///
    /// # Arguments
    /// - `random` – The random number generator to use.
    /// - `octaves` – The list of octave indices to include (can be negative).
    ///
    /// # Returns
    /// A new `OctaveSimplexNoiseSampler` instance.
    #[expect(clippy::many_single_char_names)]
    pub fn new(random: &mut impl RandomImpl, octaves: &[i32]) -> Self {
        let mut octaves = Vec::from_iter(octaves);
        octaves.sort();

        let (Some(&&first), Some(&&last)) = (octaves.first(), octaves.last()) else {
            return Self {
                octave_samplers: Vec::new(),
                persistence: 0.0,
                lacunarity: 0.0,
            };
        };
        let i = -first;
        let j = last;
        let k = i.wrapping_add(j).wrapping_add(1);

        let sampler = SimplexNoiseSampler::new(random);
        let l = j;
        let mut samplers: Vec<Option<SimplexNoiseSampler>> = Vec::with_capacity(k as usize);
        for _ in 0..k {
            samplers.push(None);
        }

        for m in (j + 1)..k {
            if m >= 0 && octaves.contains(&&(l - m)) {
                let sampler = SimplexNoiseSampler::new(random);
                samplers[m as usize] = Some(sampler);
            } else {
                random.skip(262);
            }
        }

        if j > 0 {
            let sample = sampler.sample_3d(sampler.x_origin, sampler.y_origin, sampler.z_origin);
            let n = (sample * f64::from(9.223372E18f32)) as i64;
            let mut random = LegacyRand::from_seed(n as u64);

            for o in (0..l).rev() {
                if o < k && octaves.contains(&&(l - o)) {
                    let sampler = SimplexNoiseSampler::new(&mut random);
                    samplers[o as usize] = Some(sampler);
                } else {
                    random.skip(262);
                }
            }
        }

        if j >= 0 && j < k && octaves.contains(&&0) {
            samplers[j as usize] = Some(sampler);
        }

        Self {
            octave_samplers: samplers,
            persistence: 1.0 / (2.0.pow(k) - 1.0),
            lacunarity: 2.0.pow(j),
        }
    }

    /// Samples 2D octave Simplex noise at the given coordinates.
    ///
    /// This method combines all active octaves, summing their contributions with
    /// appropriate scaling based on persistence and lacunarity.
    ///
    /// # Arguments
    /// - `x` – The X coordinate.
    /// - `y` – The Y coordinate.
    /// - `use_origin` – Whether to include the per-octave origin offsets.
    ///
    /// # Returns
    /// The combined noise value from all octaves.
    #[must_use]
    #[expect(clippy::many_single_char_names)]
    #[expect(clippy::suboptimal_flops)]
    pub fn sample(&self, x: f64, y: f64, use_origin: bool) -> f64 {
        let mut d = 0.0;
        let mut e = self.lacunarity;
        let mut f = self.persistence;

        for sampler in &self.octave_samplers {
            if let Some(sampler) = sampler {
                d += sampler.sample_2d(
                    x * e + if use_origin { sampler.x_origin } else { 0.0 },
                    y * e + if use_origin { sampler.y_origin } else { 0.0 },
                ) * f;
            }

            e /= 2.0;
            f *= 2.0;
        }

        d
    }
}
