//! Prepared pure chains. Mutable caches/interpolation/branches remain boundaries.
use super::{
    chunk_noise_router::{ChunkNoiseFunctionComponent, StaticChunkNoiseFunctionComponentImpl},
    density_function::NoiseFunctionComponentRange,
    density_volume::DensityVolume,
    proto_noise_router::{
        DependentProtoNoiseFunctionComponent as D, ProtoNoiseFunctionComponent as P,
    },
};
use pumpkin_data::noise_router::{
    BinaryOperation, ClampData, LinearData, LinearOperation, UnaryData, UnaryOperation,
};
use pumpkin_util::math::vector3::Vector3;

#[derive(Clone, Copy)]
pub enum Operation {
    Linear(&'static LinearData),
    Unary(&'static UnaryData),
    Clamp(&'static ClampData),
}
impl Operation {
    fn apply(self, value: f32) -> f32 {
        match self {
            Self::Linear(data) => data.apply_density(value),
            Self::Unary(data) => data.apply_density(value),
            Self::Clamp(data) => data.apply_density(value),
        }
    }
}

pub struct PointwisePlan {
    input: usize,
    binary: Option<(usize, BinaryOperation)>,
    operations: Box<[Operation]>,
    min: f32,
    max: f32,
}

pub(super) fn prepare(component: P, stack: &[P]) -> P {
    let (input, operation) = match &component {
        P::Dependent(D::Linear(node)) => (node.input_index, Operation::Linear(node.data)),
        P::Dependent(D::Unary(node)) => (node.input_index, Operation::Unary(node.data)),
        P::Dependent(D::Clamp(node)) => (node.input_index, Operation::Clamp(node.data)),
        _ => return component,
    };
    let (base, binary, mut operations) = match &stack[input] {
        P::Dependent(D::Pointwise(plan)) => (plan.input, plan.binary, plan.operations.to_vec()),
        P::Dependent(D::Linear(node)) => {
            (node.input_index, None, vec![Operation::Linear(node.data)])
        }
        P::Dependent(D::Unary(node)) => (node.input_index, None, vec![Operation::Unary(node.data)]),
        P::Dependent(D::Clamp(node)) => (node.input_index, None, vec![Operation::Clamp(node.data)]),
        P::Dependent(D::Binary(node))
            if matches!(
                node.data.operation,
                BinaryOperation::Add
                    | BinaryOperation::Sub
                    | BinaryOperation::Mul
                    | BinaryOperation::Div
            ) =>
        {
            (
                node.input1_index,
                Some((node.input2_index, node.data.operation)),
                Vec::new(),
            )
        }
        _ => return component,
    };
    operations.push(operation);
    P::Dependent(D::Pointwise(PointwisePlan {
        input: base,
        binary,
        operations: operations.into_boxed_slice(),
        min: component.min(),
        max: component.max(),
    }))
}

impl NoiseFunctionComponentRange for PointwisePlan {
    fn min(&self) -> f32 {
        self.min
    }
    fn max(&self) -> f32 {
        self.max
    }
}
impl StaticChunkNoiseFunctionComponentImpl for PointwisePlan {
    fn sample(&self, stack: &mut [ChunkNoiseFunctionComponent], pos: &Vector3<i32>) -> f32 {
        let mut value =
            ChunkNoiseFunctionComponent::sample_from_stack(&mut stack[..=self.input], pos);
        if let Some((right, operation)) = self.binary {
            value = if value == 0.0
                && matches!(operation, BinaryOperation::Mul | BinaryOperation::Div)
            {
                0.0
            } else {
                let right =
                    ChunkNoiseFunctionComponent::sample_from_stack(&mut stack[..=right], pos);
                combine(value, right, operation)
            };
        }
        for &operation in &self.operations {
            value = operation.apply(value);
        }
        value
    }
    fn sample_volume(
        &self,
        stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        ChunkNoiseFunctionComponent::sample_volume_from_stack(
            &mut stack[..=self.input],
            buffer,
            volume,
        );
        if let Some((right_index, operation)) = self.binary {
            if right_index == self.input {
                for value in buffer.iter_mut() {
                    *value = combine(*value, *value, operation);
                }
                apply(buffer, &self.operations);
            } else {
                let mut right = super::density_volume::DensityBuffer::acquire(volume);
                ChunkNoiseFunctionComponent::sample_volume_from_stack(
                    &mut stack[..=right_index],
                    &mut right,
                    volume,
                );
                apply_binary(buffer, &right, operation, &self.operations);
            }
        } else {
            apply(buffer, &self.operations);
        }
    }
}

pub fn apply(buffer: &mut [f32], operations: &[Operation]) {
    apply_with_binary(buffer, None, operations);
}

/// Execute one binary node followed by its unary chain without intermediate
/// stores. Inputs must be equally sized; floating-point operations stay separate.
pub fn apply_binary(
    buffer: &mut [f32],
    right: &[f32],
    operation: BinaryOperation,
    operations: &[Operation],
) {
    assert_eq!(buffer.len(), right.len());
    assert!(matches!(
        operation,
        BinaryOperation::Add | BinaryOperation::Sub | BinaryOperation::Mul | BinaryOperation::Div
    ));
    apply_with_binary(buffer, Some((right, operation)), operations);
}

fn combine(left: f32, right: f32, operation: BinaryOperation) -> f32 {
    match operation {
        BinaryOperation::Add => left + right,
        BinaryOperation::Sub => left - right,
        BinaryOperation::Mul => left * right,
        BinaryOperation::Div => left / right,
        _ => unreachable!("only arithmetic nodes are fused"),
    }
}

fn apply_with_binary(
    buffer: &mut [f32],
    binary: Option<(&[f32], BinaryOperation)>,
    operations: &[Operation],
) {
    let mut start = 0;
    #[cfg(target_arch = "x86_64")]
    if buffer.len() >= 8
        && std::is_x86_feature_detected!("avx2")
        && !operations.iter().any(|operation| {
            matches!(
                operation,
                Operation::Unary(UnaryData {
                    operation: UnaryOperation::Log
                })
            )
        })
    {
        // SAFETY: AVX2 detected; kernel handles complete groups within buffer.
        start = unsafe { apply_avx2(buffer, binary, operations) };
    }
    for (index, value) in buffer.iter_mut().enumerate().skip(start) {
        if let Some((right, operation)) = binary {
            *value = combine(*value, right[index], operation);
        }
        for &operation in operations {
            *value = operation.apply(*value);
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn apply_avx2(
    buffer: &mut [f32],
    binary: Option<(&[f32], BinaryOperation)>,
    operations: &[Operation],
) -> usize {
    use std::arch::x86_64::*;
    // SAFETY: runtime feature check and bounds in caller; no pointer escapes.
    unsafe {
        let zero = _mm256_setzero_ps();
        let one = _mm256_set1_ps(1.0);
        let clamp = |v, min, max| {
            let low = _mm256_blendv_ps(v, min, _mm256_cmp_ps::<_CMP_LT_OQ>(v, min));
            _mm256_blendv_ps(low, max, _mm256_cmp_ps::<_CMP_GT_OQ>(low, max))
        };
        let end = buffer.len() / 8 * 8;
        for index in (0..end).step_by(8) {
            let mut value = _mm256_loadu_ps(buffer.as_ptr().add(index));
            if let Some((right, operation)) = binary {
                let right = _mm256_loadu_ps(right.as_ptr().add(index));
                value = match operation {
                    BinaryOperation::Add => _mm256_add_ps(value, right),
                    BinaryOperation::Sub => _mm256_sub_ps(value, right),
                    BinaryOperation::Mul => _mm256_mul_ps(value, right),
                    BinaryOperation::Div => _mm256_div_ps(value, right),
                    _ => unreachable!("only arithmetic nodes are fused"),
                };
            }
            for operation in operations {
                value = match operation {
                    Operation::Linear(data) => match data.operation {
                        LinearOperation::Add => _mm256_add_ps(value, _mm256_set1_ps(data.argument)),
                        LinearOperation::Mul => _mm256_mul_ps(value, _mm256_set1_ps(data.argument)),
                    },
                    Operation::Clamp(data) => clamp(
                        value,
                        _mm256_set1_ps(data.min_value),
                        _mm256_set1_ps(data.max_value),
                    ),
                    Operation::Unary(data) => match data.operation {
                        UnaryOperation::Abs => _mm256_andnot_ps(_mm256_set1_ps(-0.0), value),
                        UnaryOperation::Square => _mm256_mul_ps(value, value),
                        UnaryOperation::Cube => _mm256_mul_ps(_mm256_mul_ps(value, value), value),
                        UnaryOperation::HalfNegative | UnaryOperation::QuarterNegative => {
                            let factor = if matches!(data.operation, UnaryOperation::HalfNegative) {
                                0.5
                            } else {
                                0.25
                            };
                            _mm256_blendv_ps(
                                _mm256_mul_ps(value, _mm256_set1_ps(factor)),
                                value,
                                _mm256_cmp_ps::<_CMP_GT_OQ>(value, zero),
                            )
                        }
                        UnaryOperation::Squeeze => {
                            let v = clamp(value, _mm256_set1_ps(-1.0), one);
                            _mm256_sub_ps(
                                _mm256_div_ps(v, _mm256_set1_ps(2.0)),
                                _mm256_div_ps(
                                    _mm256_mul_ps(_mm256_mul_ps(v, v), v),
                                    _mm256_set1_ps(24.0),
                                ),
                            )
                        }
                        UnaryOperation::Invert => _mm256_blendv_ps(
                            _mm256_div_ps(one, value),
                            _mm256_set1_ps(f32::INFINITY),
                            _mm256_cmp_ps::<_CMP_EQ_OQ>(value, zero),
                        ),
                        UnaryOperation::Negate => _mm256_xor_ps(value, _mm256_set1_ps(-0.0)),
                        UnaryOperation::Sqrt => _mm256_sqrt_ps(value),
                        UnaryOperation::Sign => _mm256_blendv_ps(
                            _mm256_blendv_ps(
                                zero,
                                _mm256_set1_ps(-1.0),
                                _mm256_cmp_ps::<_CMP_LT_OQ>(value, zero),
                            ),
                            one,
                            _mm256_cmp_ps::<_CMP_GT_OQ>(value, zero),
                        ),
                        UnaryOperation::Log => unreachable!("log uses the scalar chain"),
                    },
                };
            }
            _mm256_storeu_ps(buffer.as_mut_ptr().add(index), value);
        }
        end
    }
}
