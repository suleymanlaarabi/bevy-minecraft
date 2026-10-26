//! Bulk arithmetic kernels. Operation selection happens outside the voxel loop.
//! AVX2 uses separate multiply/add instructions to preserve rounding; other CPUs
//! use the same scalar operations and LLVM's portable auto-vectorization.
use pumpkin_data::noise_router::{BinaryOperation, LinearOperation};

#[inline]
fn arithmetic<const OP: u8>(a: f32, b: f32) -> f32 {
    match OP {
        0 => a + b,
        1 => a - b,
        2 => a * b,
        _ => a / b,
    }
}

fn combine<const OP: u8>(left: &mut [f32], right: &[f32]) {
    assert_eq!(left.len(), right.len());
    let mut start = 0;
    #[cfg(target_arch = "x86_64")]
    if left.len() >= 8 && std::is_x86_feature_detected!("avx2") {
        // SAFETY: runtime feature detection; the kernel only accesses complete
        // eight-element groups inside these equally-sized, disjoint slices.
        start = unsafe { combine_avx2::<OP>(left, right) };
    }
    for (a, &b) in left[start..].iter_mut().zip(&right[start..]) {
        *a = arithmetic::<OP>(*a, b);
    }
}

pub fn binary(left: &mut [f32], right: &[f32], operation: BinaryOperation) {
    assert_eq!(left.len(), right.len());
    match operation {
        BinaryOperation::Add => combine::<0>(left, right),
        BinaryOperation::Sub => combine::<1>(left, right),
        BinaryOperation::Mul => combine::<2>(left, right),
        BinaryOperation::Div => combine::<3>(left, right),
        BinaryOperation::Min => {
            for (a, &b) in left.iter_mut().zip(right) {
                *a = a.min(b);
            }
        }
        BinaryOperation::Max => {
            for (a, &b) in left.iter_mut().zip(right) {
                *a = a.max(b);
            }
        }
        BinaryOperation::Pow => {
            for (a, &b) in left.iter_mut().zip(right) {
                *a = f64::from(*a).powf(f64::from(b)) as f32;
            }
        }
    }
}

fn linear_pair_kernel<const FIRST_MUL: bool, const SECOND_MUL: bool>(
    buffer: &mut [f32],
    first: f32,
    second: f32,
) {
    let mut start = 0;
    #[cfg(target_arch = "x86_64")]
    if buffer.len() >= 8 && std::is_x86_feature_detected!("avx2") {
        // SAFETY: feature checked above; only in-bounds groups of eight are used.
        start = unsafe { linear_pair_avx2::<FIRST_MUL, SECOND_MUL>(buffer, first, second) };
    }
    for value in &mut buffer[start..] {
        let value1 = if FIRST_MUL {
            *value * first
        } else {
            *value + first
        };
        *value = if SECOND_MUL {
            value1 * second
        } else {
            value1 + second
        };
    }
}

/// Fuse two consecutive linear nodes without reassociating their arithmetic.
pub fn linear_pair(
    buffer: &mut [f32],
    first: LinearOperation,
    first_arg: f32,
    second: LinearOperation,
    second_arg: f32,
) {
    match (first, second) {
        (LinearOperation::Add, LinearOperation::Add) => {
            linear_pair_kernel::<false, false>(buffer, first_arg, second_arg)
        }
        (LinearOperation::Add, LinearOperation::Mul) => {
            linear_pair_kernel::<false, true>(buffer, first_arg, second_arg)
        }
        (LinearOperation::Mul, LinearOperation::Add) => {
            linear_pair_kernel::<true, false>(buffer, first_arg, second_arg)
        }
        (LinearOperation::Mul, LinearOperation::Mul) => {
            linear_pair_kernel::<true, true>(buffer, first_arg, second_arg)
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn combine_avx2<const OP: u8>(left: &mut [f32], right: &[f32]) -> usize {
    use std::arch::x86_64::*;
    let end = left.len() / 8 * 8;
    for i in (0..end).step_by(8) {
        // SAFETY: callers check equal lengths; i..i+8 is inside both slices.
        unsafe {
            let a = _mm256_loadu_ps(left.as_ptr().add(i));
            let b = _mm256_loadu_ps(right.as_ptr().add(i));
            let value = match OP {
                0 => _mm256_add_ps(a, b),
                1 => _mm256_sub_ps(a, b),
                2 => _mm256_mul_ps(a, b),
                _ => _mm256_div_ps(a, b),
            };
            _mm256_storeu_ps(left.as_mut_ptr().add(i), value);
        }
    }
    end
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn linear_pair_avx2<const FIRST_MUL: bool, const SECOND_MUL: bool>(
    buffer: &mut [f32],
    first: f32,
    second: f32,
) -> usize {
    use std::arch::x86_64::*;
    let end = buffer.len() / 8 * 8;
    let first = _mm256_set1_ps(first);
    let second = _mm256_set1_ps(second);
    for i in (0..end).step_by(8) {
        // SAFETY: i..i+8 is in bounds; unaligned accesses are supported.
        unsafe {
            let value = _mm256_loadu_ps(buffer.as_ptr().add(i));
            let value1 = if FIRST_MUL {
                _mm256_mul_ps(value, first)
            } else {
                _mm256_add_ps(value, first)
            };
            let result = if SECOND_MUL {
                _mm256_mul_ps(value1, second)
            } else {
                _mm256_add_ps(value1, second)
            };
            _mm256_storeu_ps(buffer.as_mut_ptr().add(i), result);
        }
    }
    end
}
