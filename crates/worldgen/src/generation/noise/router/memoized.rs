//! Memoize expensive shared DAG nodes on small grids, with a strict per-node cap.
//! Point sampling retains its own arithmetic, independently of volume caches.
use super::{
    chunk_noise_router::{ChunkNoiseFunctionComponent, StaticChunkNoiseFunctionComponentImpl},
    density_volume::{DensityBuffer, DensityVolume},
    proto_noise_router::DependentProtoNoiseFunctionComponent,
};
use pumpkin_util::math::vector3::Vector3;

pub(super) fn reference_counts(
    stack: &[pumpkin_data::noise_router::BaseNoiseFunctionComponent],
) -> Vec<usize> {
    use pumpkin_data::noise_router::{BaseNoiseFunctionComponent as B, SplineRepr};
    fn spline(value: &SplineRepr, counts: &mut [usize]) {
        if let SplineRepr::Standard {
            location_function_index,
            points,
        } = value
        {
            counts[*location_function_index] += 1;
            for point in *points {
                spline(point.value, counts);
            }
        }
    }
    let mut counts = vec![0; stack.len()];
    for component in stack {
        match component {
            B::Spline { spline: value } => spline(value, &mut counts),
            B::BlendDensity { input_index }
            | B::Wrapper { input_index, .. }
            | B::Linear { input_index, .. }
            | B::Unary { input_index, .. }
            | B::Clamp { input_index, .. }
            | B::Slice { input_index, .. } => counts[*input_index] += 1,
            B::FindTopSurface {
                density_index,
                upper_bound_index,
                ..
            } => {
                counts[*density_index] += 1;
                counts[*upper_bound_index] += 1;
            }
            B::ShiftedNoise {
                shift_x_index,
                shift_y_index,
                shift_z_index,
                ..
            } => {
                for index in [shift_x_index, shift_y_index, shift_z_index] {
                    counts[*index] += 1;
                }
            }
            B::IntervalSelect {
                input_index,
                functions_indices,
                ..
            } => {
                counts[*input_index] += 1;
                for index in *functions_indices {
                    counts[*index] += 1;
                }
            }
            B::Lerp {
                alpha_index,
                first_index,
                second_index,
            } => {
                for index in [alpha_index, first_index, second_index] {
                    counts[*index] += 1;
                }
            }
            B::Rounding {
                input_index,
                multiple_index,
                ..
            } => {
                counts[*input_index] += 1;
                counts[*multiple_index] += 1;
            }
            B::Binary {
                argument1_index,
                argument2_index,
                ..
            } => {
                counts[*argument1_index] += 1;
                counts[*argument2_index] += 1;
            }
            B::RangeChoice {
                input_index,
                when_in_range_index,
                when_out_range_index,
                ..
            } => {
                for index in [input_index, when_in_range_index, when_out_range_index] {
                    counts[*index] += 1;
                }
            }
            _ => {}
        }
    }
    counts
}

pub struct MemoizedDependent<'a> {
    pub(crate) function: &'a DependentProtoNoiseFunctionComponent,
    volume: Option<DensityVolume>,
    buffer: Option<DensityBuffer>,
}

impl<'a> MemoizedDependent<'a> {
    pub const fn new(function: &'a DependentProtoNoiseFunctionComponent) -> Self {
        Self {
            function,
            volume: None,
            buffer: None,
        }
    }

    pub fn sample(&self, stack: &mut [ChunkNoiseFunctionComponent], pos: &Vector3<i32>) -> f32 {
        self.function.sample(stack, pos)
    }

    pub fn sample_volume(
        &mut self,
        stack: &mut [ChunkNoiseFunctionComponent],
        output: &mut [f32],
        volume: &DensityVolume,
    ) {
        // At most sixteen nodes are selected when preparing a router: 512 KiB
        // of logical f32 data, or 1 MiB including buffer-pool capacity rounding.
        if output.len() > 8192 {
            self.function.sample_volume(stack, output, volume);
            return;
        }
        if self.volume != Some(*volume) {
            self.buffer = None;
            let mut buffer = DensityBuffer::acquire(volume);
            self.function.sample_volume(stack, &mut buffer, volume);
            self.buffer = Some(buffer);
            self.volume = Some(*volume);
        }
        output.copy_from_slice(self.buffer.as_ref().unwrap());
    }
}
