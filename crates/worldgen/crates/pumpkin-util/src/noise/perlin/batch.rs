//! Exact SIMD kernels: f64 coordinates, f32 gradients and separate mul/add.
use super::{
    Corners, GRADIENTS, HALF_ROUND_OFF, Noise, PerlinNoise, ROUND_OFF, SmearedPerlinNoise, wrap,
};
use crate::{math::smoothstep, noise::volume::DensityVolume};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseBackend {
    Scalar,
    Avx2,
}

pub fn selected_backend() -> NoiseBackend {
    static BACKEND: OnceLock<NoiseBackend> = OnceLock::new();
    *BACKEND.get_or_init(|| {
        if std::env::var("WORLDGEN_NOISE_BACKEND").as_deref() != Ok("scalar") {
            #[cfg(target_arch = "x86_64")]
            if std::is_x86_feature_detected!("avx2") {
                return NoiseBackend::Avx2;
            }
        }
        NoiseBackend::Scalar
    })
}

pub(super) fn get_many(noise: &PerlinNoise, positions: &[[f64; 3]], output: &mut [f32]) {
    assert_eq!(positions.len(), output.len());
    let mut start = 0;
    #[cfg(target_arch = "x86_64")]
    if selected_backend() == NoiseBackend::Avx2 {
        for (positions, output) in positions.chunks_exact(8).zip(output.chunks_exact_mut(8)) {
            // SAFETY: AVX2 was checked at runtime. The kernel reads exactly eight
            // positions and only gathers masked permutation/gradient indices.
            unsafe {
                sample8(
                    noise,
                    positions.try_into().unwrap(),
                    output.try_into().unwrap(),
                );
            }
            start += 8;
        }
    }
    for (&[x, y, z], value) in positions[start..].iter().zip(&mut output[start..]) {
        *value = noise.get(x, y, z);
    }
}

pub(super) const MAX_GRID_HEIGHT: usize = 512;

/// Hoist all Y coordinate/wrapping/fudge work out of the X/Z column loops.
/// Keep the original corner reuse; vectorize only runs sharing those corners.
pub(super) fn add_to_grid(
    noise: &PerlinNoise,
    smeared: Option<&SmearedPerlinNoise>,
    mut buffer: &mut [f32],
    volume: &DensityVolume,
    xz_scale: f64,
    y_scale: f64,
    amplitude: f32,
) {
    assert_eq!(buffer.len(), volume.size());
    let mut floors = [0; MAX_GRID_HEIGHT];
    let mut relative = [0.0; MAX_GRID_HEIGHT];
    let mut alpha = [0.0; MAX_GRID_HEIGHT];
    for index in 0..volume.size_y {
        let original = f64::from(volume.block_y(index)) * y_scale;
        let y = wrap(original) + noise.offset_y;
        floors[index] = y.floor() as i32;
        let fraction = y - f64::from(floors[index]);
        alpha[index] = smoothstep(fraction as f32);
        relative[index] = if let Some(smeared) = smeared {
            (fraction - smeared.compute_fudge_y(original, fraction)) as f32
        } else {
            fraction as f32
        };
    }
    let mut corners = Corners::zero();
    for z in 0..volume.size_z {
        let z = wrap(f64::from(volume.block_z(z)) * xz_scale) + noise.offset_z;
        let floor_z = z.floor() as i32;
        let relative_z = (z - f64::from(floor_z)) as f32;
        let alpha_z = smoothstep(relative_z);
        for x in 0..volume.size_x {
            let x = wrap(f64::from(volume.block_x(x)) * xz_scale) + noise.offset_x;
            let floor_x = x.floor() as i32;
            let relative_x = (x - f64::from(floor_x)) as f32;
            let alpha_x = smoothstep(relative_x);
            let x0 = noise.permute(floor_x);
            let x1 = noise.permute(floor_x + 1);
            let (column, remaining) = buffer.split_at_mut(volume.size_y);
            buffer = remaining;
            let mut start = 0;
            while start < volume.size_y {
                let mut end = start + 1;
                while end < volume.size_y && floors[end] == floors[start] {
                    end += 1;
                }
                noise.corners(
                    x0,
                    x1,
                    floors[start],
                    floor_z,
                    relative_x,
                    relative_z,
                    &mut corners,
                );
                let mut index = start;
                #[cfg(target_arch = "x86_64")]
                if end - start >= 8 {
                    // SAFETY: this entry point is selected only after runtime
                    // AVX2 detection; all three slices describe the same run.
                    index += unsafe {
                        avx::add_corners(
                            &corners,
                            alpha_x,
                            alpha_z,
                            amplitude,
                            &relative[start..end],
                            &alpha[start..end],
                            &mut column[start..end],
                        )
                    };
                }
                for i in index..end {
                    column[i] += amplitude * corners.lerp(alpha_x, alpha[i], alpha_z, relative[i]);
                }
                start = end;
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod avx {
    use super::*;
    use std::arch::x86_64::*;

    const fn gradient_axis(axis: usize) -> [i8; 16] {
        let mut values = [0; 16];
        let mut i = 0;
        while i < 16 {
            values[i] = GRADIENTS[i][axis] as i8;
            i += 1;
        }
        values
    }
    const GX: [i8; 16] = gradient_axis(0);
    const GY: [i8; 16] = gradient_axis(1);
    const GZ: [i8; 16] = gradient_axis(2);

    #[target_feature(enable = "avx2")]
    unsafe fn wrap4(value: __m256d) -> __m256d {
        let keep = _mm256_and_pd(
            _mm256_cmp_pd::<_CMP_GE_OQ>(value, _mm256_set1_pd(-HALF_ROUND_OFF)),
            _mm256_cmp_pd::<_CMP_LT_OQ>(value, _mm256_set1_pd(HALF_ROUND_OFF)),
        );
        let turns = _mm256_floor_pd(_mm256_add_pd(
            _mm256_div_pd(value, _mm256_set1_pd(ROUND_OFF)),
            _mm256_set1_pd(0.5),
        ));
        _mm256_blendv_pd(
            _mm256_sub_pd(value, _mm256_mul_pd(turns, _mm256_set1_pd(ROUND_OFF))),
            value,
            keep,
        )
    }

    #[target_feature(enable = "avx2")]
    unsafe fn coordinate(base: *const f64, axis: usize, offset: f64) -> (__m256i, __m256) {
        // SAFETY: sample8 supplies eight [f64; 3] positions. The gathers read
        // axis + {0,3,6,9} and axis + {12,15,18,21}, all in that array.
        unsafe {
            let indices = _mm_setr_epi32(0, 3, 6, 9);
            let low = _mm256_add_pd(
                wrap4(_mm256_i32gather_pd::<8>(base.add(axis), indices)),
                _mm256_set1_pd(offset),
            );
            let high = _mm256_add_pd(
                wrap4(_mm256_i32gather_pd::<8>(base.add(axis + 12), indices)),
                _mm256_set1_pd(offset),
            );
            let floor_low = _mm256_floor_pd(low);
            let floor_high = _mm256_floor_pd(high);
            let integer = _mm256_inserti128_si256::<1>(
                _mm256_castsi128_si256(_mm256_cvttpd_epi32(floor_low)),
                _mm256_cvttpd_epi32(floor_high),
            );
            let fraction = _mm256_insertf128_ps::<1>(
                _mm256_castps128_ps256(_mm256_cvtpd_ps(_mm256_sub_pd(low, floor_low))),
                _mm256_cvtpd_ps(_mm256_sub_pd(high, floor_high)),
            );
            (integer, fraction)
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn permute(noise: &PerlinNoise, index: __m256i) -> __m256i {
        // SAFETY: masking limits every gather to the 256-entry permutation table.
        unsafe {
            _mm256_i32gather_epi32::<4>(
                noise.perms_wide.as_ptr(),
                _mm256_and_si256(index, _mm256_set1_epi32(255)),
            )
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn gradient(hash: __m256i, x: __m256, y: __m256, z: __m256) -> __m256 {
        let index = _mm256_and_si256(hash, _mm256_set1_epi32(15));
        // SAFETY: each coefficient table contains 16 bytes. Byte shuffles keep
        // lookup tables in registers instead of gathering 24 gradient arrays.
        unsafe {
            let coefficient = |table: &[i8; 16]| {
                let table = _mm256_broadcastsi128_si256(_mm_loadu_si128(table.as_ptr().cast()));
                let bytes = _mm256_shuffle_epi8(table, index);
                _mm256_cvtepi32_ps(_mm256_srai_epi32::<24>(_mm256_slli_epi32::<24>(bytes)))
            };
            _mm256_add_ps(
                _mm256_add_ps(
                    _mm256_mul_ps(coefficient(&GX), x),
                    _mm256_mul_ps(coefficient(&GY), y),
                ),
                _mm256_mul_ps(coefficient(&GZ), z),
            )
        }
    }

    #[target_feature(enable = "avx2")]
    unsafe fn smooth(x: __m256) -> __m256 {
        _mm256_mul_ps(
            _mm256_mul_ps(_mm256_mul_ps(x, x), x),
            _mm256_add_ps(
                _mm256_mul_ps(
                    x,
                    _mm256_sub_ps(_mm256_mul_ps(x, _mm256_set1_ps(6.0)), _mm256_set1_ps(15.0)),
                ),
                _mm256_set1_ps(10.0),
            ),
        )
    }

    #[target_feature(enable = "avx2")]
    unsafe fn lerp(t: __m256, a: __m256, b: __m256) -> __m256 {
        _mm256_add_ps(a, _mm256_mul_ps(t, _mm256_sub_ps(b, a)))
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn add_corners(
        c: &Corners,
        ax: f32,
        az: f32,
        amplitude: f32,
        relative: &[f32],
        alpha: &[f32],
        output: &mut [f32],
    ) -> usize {
        // SAFETY: caller provides equally sized slices, AVX2, and full lanes only.
        unsafe {
            let end = output.len() / 8 * 8;
            let ax = _mm256_set1_ps(ax);
            let az = _mm256_set1_ps(az);
            let amplitude = _mm256_set1_ps(amplitude);
            for i in (0..end).step_by(8) {
                let y = _mm256_loadu_ps(relative.as_ptr().add(i));
                let ym = _mm256_sub_ps(y, _mm256_set1_ps(1.0));
                let ay = _mm256_loadu_ps(alpha.as_ptr().add(i));
                let dot = |xz: f32, gy: f32, ry: __m256| {
                    _mm256_add_ps(_mm256_set1_ps(xz), _mm256_mul_ps(_mm256_set1_ps(gy), ry))
                };
                let result = lerp(
                    az,
                    lerp(
                        ay,
                        lerp(ax, dot(c.d000_xz, c.g000_y, y), dot(c.d100_xz, c.g100_y, y)),
                        lerp(
                            ax,
                            dot(c.d010_xz, c.g010_y, ym),
                            dot(c.d110_xz, c.g110_y, ym),
                        ),
                    ),
                    lerp(
                        ay,
                        lerp(ax, dot(c.d001_xz, c.g001_y, y), dot(c.d101_xz, c.g101_y, y)),
                        lerp(
                            ax,
                            dot(c.d011_xz, c.g011_y, ym),
                            dot(c.d111_xz, c.g111_y, ym),
                        ),
                    ),
                );
                let previous = _mm256_loadu_ps(output.as_ptr().add(i));
                _mm256_storeu_ps(
                    output.as_mut_ptr().add(i),
                    _mm256_add_ps(previous, _mm256_mul_ps(amplitude, result)),
                );
            }
            end
        }
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn sample8(
        noise: &PerlinNoise,
        positions: &[[f64; 3]; 8],
        output: &mut [f32; 8],
    ) {
        // SAFETY: caller verifies AVX2 and fixed array lengths; helper gathers
        // are masked to their respective lookup tables.
        unsafe {
            let base = positions.as_ptr().cast::<f64>();
            let (ix, x) = coordinate(base, 0, noise.offset_x);
            let (iy, y) = coordinate(base, 1, noise.offset_y);
            let (iz, z) = coordinate(base, 2, noise.offset_z);
            let one = _mm256_set1_epi32(1);
            let x0 = permute(noise, ix);
            let x1 = permute(noise, _mm256_add_epi32(ix, one));
            let xy00 = permute(noise, _mm256_add_epi32(x0, iy));
            let xy01 = permute(noise, _mm256_add_epi32(_mm256_add_epi32(x0, iy), one));
            let xy10 = permute(noise, _mm256_add_epi32(x1, iy));
            let xy11 = permute(noise, _mm256_add_epi32(_mm256_add_epi32(x1, iy), one));
            let xm = _mm256_sub_ps(x, _mm256_set1_ps(1.0));
            let ym = _mm256_sub_ps(y, _mm256_set1_ps(1.0));
            let zm = _mm256_sub_ps(z, _mm256_set1_ps(1.0));
            let d000 = gradient(permute(noise, _mm256_add_epi32(xy00, iz)), x, y, z);
            let d100 = gradient(permute(noise, _mm256_add_epi32(xy10, iz)), xm, y, z);
            let d010 = gradient(permute(noise, _mm256_add_epi32(xy01, iz)), x, ym, z);
            let d110 = gradient(permute(noise, _mm256_add_epi32(xy11, iz)), xm, ym, z);
            let iz1 = _mm256_add_epi32(iz, one);
            let d001 = gradient(permute(noise, _mm256_add_epi32(xy00, iz1)), x, y, zm);
            let d101 = gradient(permute(noise, _mm256_add_epi32(xy10, iz1)), xm, y, zm);
            let d011 = gradient(permute(noise, _mm256_add_epi32(xy01, iz1)), x, ym, zm);
            let d111 = gradient(permute(noise, _mm256_add_epi32(xy11, iz1)), xm, ym, zm);
            let ax = smooth(x);
            let ay = smooth(y);
            let az = smooth(z);
            let result = lerp(
                az,
                lerp(ay, lerp(ax, d000, d100), lerp(ax, d010, d110)),
                lerp(ay, lerp(ax, d001, d101), lerp(ax, d011, d111)),
            );
            _mm256_storeu_ps(output.as_mut_ptr(), result);
        }
    }
}

#[cfg(target_arch = "x86_64")]
use avx::sample8;
