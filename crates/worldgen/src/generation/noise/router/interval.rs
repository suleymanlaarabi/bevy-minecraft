//! Conservative value bounds of a density graph over an axis-aligned block box.
//!
//! Interpolated nodes bound their trilinear blend by the extreme corner values
//! of the cells covering the box; arithmetic nodes propagate those bounds. Any
//! other node falls back to its static range, so the result always contains
//! every value the graph can produce inside the box. Float rounding is
//! monotonic for the propagated operations; callers add a small margin for the
//! interpolation lerps themselves.
use pumpkin_data::noise_router::{BinaryOperation, LinearOperation, UnaryOperation};

use super::{
    chunk_density_function::ChunkSpecificNoiseFunctionComponent,
    chunk_noise_router::ChunkNoiseFunctionComponent, density_function::NoiseFunctionComponentRange,
    pointwise::Operation, proto_noise_router::DependentProtoNoiseFunctionComponent as D,
};

pub type Bounds = (f32, f32);

/// Inclusive block box.
#[derive(Clone, Copy)]
pub struct BlockBounds {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

const UNBOUNDED: Bounds = (f32::NEG_INFINITY, f32::INFINITY);

fn sanitize((lo, hi): Bounds) -> Bounds {
    if lo.is_nan() || hi.is_nan() || lo > hi {
        UNBOUNDED
    } else {
        (lo, hi)
    }
}

fn hull((a_lo, a_hi): Bounds, (b_lo, b_hi): Bounds) -> Bounds {
    (a_lo.min(b_lo), a_hi.max(b_hi))
}

fn monotonic((lo, hi): Bounds, apply: impl Fn(f32) -> f32) -> Bounds {
    (apply(lo), apply(hi))
}

pub(crate) fn linear(bounds: Bounds, operation: LinearOperation, argument: f32) -> Bounds {
    match operation {
        LinearOperation::Add => monotonic(bounds, |v| v + argument),
        LinearOperation::Mul if argument >= 0.0 => monotonic(bounds, |v| v * argument),
        LinearOperation::Mul => (bounds.1 * argument, bounds.0 * argument),
    }
}

pub(crate) fn unary(bounds @ (lo, hi): Bounds, operation: UnaryOperation) -> Option<Bounds> {
    Some(match operation {
        UnaryOperation::Abs | UnaryOperation::Square => {
            let (lo, hi) = if lo >= 0.0 {
                (lo, hi)
            } else if hi <= 0.0 {
                (-hi, -lo)
            } else {
                (0.0, (-lo).max(hi))
            };
            if matches!(operation, UnaryOperation::Square) {
                (lo * lo, hi * hi)
            } else {
                (lo, hi)
            }
        }
        UnaryOperation::Cube => monotonic(bounds, |v| v * v * v),
        UnaryOperation::HalfNegative => monotonic(bounds, |v| if v > 0.0 { v } else { v * 0.5 }),
        UnaryOperation::QuarterNegative => {
            monotonic(bounds, |v| if v > 0.0 { v } else { v * 0.25 })
        }
        UnaryOperation::Squeeze => monotonic(bounds, |v| {
            let c = v.clamp(-1.0, 1.0);
            c / 2.0 - c * c * c / 24.0
        }),
        UnaryOperation::Negate => (-hi, -lo),
        UnaryOperation::Sign => monotonic(bounds, |v| {
            if v > 0.0 {
                1.0
            } else if v < 0.0 {
                -1.0
            } else {
                0.0
            }
        }),
        UnaryOperation::Sqrt if lo >= 0.0 => monotonic(bounds, f32::sqrt),
        UnaryOperation::Log if lo > 0.0 => monotonic(bounds, f32::ln),
        UnaryOperation::Invert | UnaryOperation::Sqrt | UnaryOperation::Log => return None,
    })
}

pub(crate) fn binary(
    (a_lo, a_hi): Bounds,
    (b_lo, b_hi): Bounds,
    operation: BinaryOperation,
) -> Option<Bounds> {
    Some(match operation {
        BinaryOperation::Add => (a_lo + b_lo, a_hi + b_hi),
        BinaryOperation::Sub => (a_lo - b_hi, a_hi - b_lo),
        BinaryOperation::Mul => {
            let products = [a_lo * b_lo, a_lo * b_hi, a_hi * b_lo, a_hi * b_hi];
            (
                products.into_iter().fold(f32::INFINITY, f32::min),
                products.into_iter().fold(f32::NEG_INFINITY, f32::max),
            )
        }
        BinaryOperation::Min => (a_lo.min(b_lo), a_hi.min(b_hi)),
        BinaryOperation::Max => (a_lo.max(b_lo), a_hi.max(b_hi)),
        BinaryOperation::Div | BinaryOperation::Pow => return None,
    })
}

pub(crate) fn operation(bounds: Bounds, operation: Operation) -> Option<Bounds> {
    match operation {
        Operation::Linear(data) => Some(linear(bounds, data.operation, data.argument)),
        Operation::Unary(data) => unary(bounds, data.operation),
        Operation::Clamp(data) => Some((
            bounds.0.clamp(data.min_value, data.max_value),
            bounds.1.clamp(data.min_value, data.max_value),
        )),
    }
}

/// Bounds of `stack[index]` over `area`.
pub fn bounds(stack: &[ChunkNoiseFunctionComponent], index: usize, area: &BlockBounds) -> Bounds {
    let component = &stack[index];
    let fallback = (component.min(), component.max());
    let derived = match component {
        ChunkNoiseFunctionComponent::Independent(_) => None,
        ChunkNoiseFunctionComponent::Dependent(dependent) => {
            dependent_bounds(stack, dependent, area)
        }
        ChunkNoiseFunctionComponent::Memoized(memoized) => {
            dependent_bounds(stack, memoized.function, area)
        }
        ChunkNoiseFunctionComponent::PassThrough(pass_through) => {
            Some(bounds(stack, pass_through.input_index(), area))
        }
        ChunkNoiseFunctionComponent::Chunk(chunk) => match chunk {
            ChunkSpecificNoiseFunctionComponent::Cache(cache) => {
                Some(bounds(stack, cache.input_index, area))
            }
            ChunkSpecificNoiseFunctionComponent::Interpolated(interpolated) => {
                interpolated.corner_bounds(area)
            }
            ChunkSpecificNoiseFunctionComponent::Beardifier(beardifier) => {
                beardifier.is_zero_inside(area).then_some((0.0, 0.0))
            }
        },
    };
    derived.map_or(fallback, sanitize)
}

fn dependent_bounds(
    stack: &[ChunkNoiseFunctionComponent],
    dependent: &D,
    area: &BlockBounds,
) -> Option<Bounds> {
    let bounds = |index| bounds(stack, index, area);
    match dependent {
        D::Pointwise(plan) => {
            let mut value = bounds(plan.input());
            if let Some((right, operation)) = plan.binary() {
                value = binary(value, bounds(right), operation)?;
            }
            for &step in plan.operations() {
                value = operation(value, step)?;
            }
            Some(value)
        }
        D::Linear(node) => Some(linear(
            bounds(node.input_index),
            node.data.operation,
            node.data.argument,
        )),
        D::Unary(node) => unary(bounds(node.input_index), node.data.operation),
        D::Clamp(node) => operation(bounds(node.input_index), Operation::Clamp(node.data)),
        D::Binary(node) => binary(
            bounds(node.input1_index),
            bounds(node.input2_index),
            node.data.operation,
        ),
        D::RangeChoice(node) => {
            let (lo, hi) = bounds(node.input_index);
            let (min, max) = (node.data.min_inclusive, node.data.max_exclusive);
            Some(if lo >= min && hi < max {
                bounds(node.when_in_index)
            } else if hi < min || lo >= max {
                bounds(node.when_out_index)
            } else {
                hull(bounds(node.when_in_index), bounds(node.when_out_index))
            })
        }
        D::Memoized(inner) => dependent_bounds(stack, inner, area),
        _ => None,
    }
}
