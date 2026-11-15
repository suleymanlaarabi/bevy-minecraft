use pumpkin_data::noise_router::WrapperType;
use pumpkin_util::math::vector3::Vector3;

use super::{
    chunk_density_function::{
        Cache, ChunkNoiseFunctionBuilderOptions, ChunkSpecificNoiseFunctionComponent, Interpolated,
    },
    density_function::{
        NoiseFunctionComponentRange, PassThrough, StaticIndependentChunkNoiseFunctionComponentImpl,
        beardifier::Beardifier,
    },
    density_volume::DensityVolume,
    proto_noise_router::{
        BEARDIFIER_ZERO_CONSTANT, DependentProtoNoiseFunctionComponent,
        IndependentProtoNoiseFunctionComponent, ProtoNoiseFunctionComponent, ProtoNoiseRouter,
    },
};

pub trait StaticChunkNoiseFunctionComponentImpl {
    fn sample(
        &self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        pos: &Vector3<i32>,
    ) -> f32;

    fn sample_volume(
        &self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        volume.fill_with(buffer, |pos| self.sample(component_stack, pos));
    }
}

pub trait MutableChunkNoiseFunctionComponentImpl {
    fn sample(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        pos: &Vector3<i32>,
    ) -> f32;

    fn sample_volume(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        volume.fill_with(buffer, |pos| self.sample(component_stack, pos));
    }
}

pub enum ChunkNoiseFunctionComponent<'a> {
    Independent(&'a IndependentProtoNoiseFunctionComponent),
    Dependent(&'a DependentProtoNoiseFunctionComponent),
    Memoized(super::memoized::MemoizedDependent<'a>),
    Chunk(ChunkSpecificNoiseFunctionComponent),
    PassThrough(PassThrough),
}

impl NoiseFunctionComponentRange for ChunkNoiseFunctionComponent<'_> {
    #[inline]
    fn min(&self) -> f32 {
        match self {
            Self::Independent(independent) => independent.min(),
            Self::Dependent(dependent) => dependent.min(),
            Self::Memoized(memoized) => memoized.function.min(),
            Self::Chunk(chunk) => chunk.min(),
            Self::PassThrough(pass_through) => pass_through.min(),
        }
    }

    #[inline]
    fn max(&self) -> f32 {
        match self {
            Self::Independent(independent) => independent.max(),
            Self::Dependent(dependent) => dependent.max(),
            Self::Memoized(memoized) => memoized.function.max(),
            Self::Chunk(chunk) => chunk.max(),
            Self::PassThrough(pass_through) => pass_through.max(),
        }
    }
}

impl MutableChunkNoiseFunctionComponentImpl for ChunkNoiseFunctionComponent<'_> {
    #[inline]
    fn sample(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        pos: &Vector3<i32>,
    ) -> f32 {
        match self {
            Self::Independent(independent) => independent.sample(pos),
            Self::Dependent(dependent) => dependent.sample(component_stack, pos),
            Self::Memoized(memoized) => memoized.sample(component_stack, pos),
            Self::Chunk(chunk) => chunk.sample(component_stack, pos),
            Self::PassThrough(pass_through) => ChunkNoiseFunctionComponent::sample_from_stack(
                &mut component_stack[..=pass_through.input_index()],
                pos,
            ),
        }
    }

    #[inline]
    fn sample_volume(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        match self {
            Self::Independent(independent) => independent.sample_volume(buffer, volume),
            Self::Dependent(dependent) => dependent.sample_volume(component_stack, buffer, volume),
            Self::Memoized(memoized) => memoized.sample_volume(component_stack, buffer, volume),
            Self::Chunk(chunk) => chunk.sample_volume(component_stack, buffer, volume),
            Self::PassThrough(pass_through) => {
                ChunkNoiseFunctionComponent::sample_volume_from_stack(
                    &mut component_stack[..=pass_through.input_index()],
                    buffer,
                    volume,
                );
            }
        }
    }
}

impl<'a> ChunkNoiseFunctionComponent<'a> {
    pub(crate) fn from_dependent(dependent: &'a DependentProtoNoiseFunctionComponent) -> Self {
        if let DependentProtoNoiseFunctionComponent::Memoized(function) = dependent {
            Self::Memoized(super::memoized::MemoizedDependent::new(function))
        } else {
            Self::Dependent(dependent)
        }
    }
    #[inline]
    pub fn sample_from_stack(
        component_stack: &mut [ChunkNoiseFunctionComponent],
        pos: &Vector3<i32>,
    ) -> f32 {
        let Some((top_component, component_stack)) = component_stack.split_last_mut() else {
            return 0.0;
        };
        top_component.sample(component_stack, pos)
    }

    pub fn sample_volume_from_stack(
        component_stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        if let Some((top_component, component_stack)) = component_stack.split_last_mut() {
            top_component.sample_volume(component_stack, buffer, volume);
        }
    }
}

pub struct ChunkNoiseDensityFunction<'a> {
    pub(crate) component_stack: &'a mut [ChunkNoiseFunctionComponent<'a>],
}

impl ChunkNoiseDensityFunction<'_> {
    #[inline]
    pub fn sample(&mut self, pos: &Vector3<i32>) -> f32 {
        ChunkNoiseFunctionComponent::sample_from_stack(self.component_stack, pos)
    }

    #[inline]
    pub fn sample_volume(&mut self, buffer: &mut [f32], volume: &DensityVolume) {
        ChunkNoiseFunctionComponent::sample_volume_from_stack(self.component_stack, buffer, volume);
    }
}

macro_rules! sample_function {
    ($name:ident, $volume_name:ident) => {
        #[inline]
        pub fn $name(&mut self, pos: &Vector3<i32>) -> f32 {
            ChunkNoiseFunctionComponent::sample_from_stack(
                &mut self.component_stack[..=self.$name],
                pos,
            )
        }

        #[inline]
        pub fn $volume_name(&mut self, buffer: &mut [f32], volume: &DensityVolume) {
            ChunkNoiseFunctionComponent::sample_volume_from_stack(
                &mut self.component_stack[..=self.$name],
                buffer,
                volume,
            );
        }
    };
}

pub struct ChunkNoiseRouter<'a> {
    barrier_noise: usize,
    fluid_level_floodedness_noise: usize,
    fluid_level_spread_noise: usize,
    lava_noise: usize,
    erosion: usize,
    depth: usize,
    final_density: usize,
    vein_toggle: usize,
    vein_ridged: usize,
    vein_gap: usize,
    component_stack: Box<[ChunkNoiseFunctionComponent<'a>]>,
}

impl ChunkNoiseRouter<'_> {
    sample_function!(barrier_noise, barrier_noise_volume);
    sample_function!(
        fluid_level_floodedness_noise,
        fluid_level_floodedness_noise_volume
    );
    sample_function!(fluid_level_spread_noise, fluid_level_spread_noise_volume);
    sample_function!(lava_noise, lava_noise_volume);
    sample_function!(erosion, erosion_volume);
    sample_function!(depth, depth_volume);
    sample_function!(final_density, final_density_volume);
    sample_function!(vein_toggle, vein_toggle_volume);
    sample_function!(vein_ridged, vein_ridged_volume);
    sample_function!(vein_gap, vein_gap_volume);
}

impl<'a> ChunkNoiseRouter<'a> {
    /// Double horizontal and vertical cell sizes once before sampling.
    /// Alignment uses world coordinates, so adjacent
    /// chunks interpolate the same shared cell corners, including negative Y.
    pub fn coarsen_interpolation(&mut self) {
        for component in &mut self.component_stack {
            if let ChunkNoiseFunctionComponent::Chunk(
                ChunkSpecificNoiseFunctionComponent::Interpolated(interpolated),
            ) = component
            {
                interpolated.coarsen();
            }
        }
    }

    /// Evaluate the corner grids of every interpolator below `final_density`
    /// once for `volume`, so block sub-volumes and bounds reuse them. Columns
    /// shared with neighboring chunks come from, and go to, `columns`.
    pub(crate) fn prepare_final_density(
        &mut self,
        volume: &DensityVolume,
        columns: Option<&super::corner_cache::CornerColumnCache>,
    ) {
        if let Some(columns) = columns
            && self.prepare_shared_columns(volume, columns)
        {
            return;
        }
        for index in 0..=self.final_density {
            let (before, rest) = self.component_stack.split_at_mut(index);
            if let ChunkNoiseFunctionComponent::Chunk(
                ChunkSpecificNoiseFunctionComponent::Interpolated(interpolated),
            ) = &mut rest[0]
            {
                interpolated.prepare(before, volume);
            }
        }
    }

    fn interpolated(&self, index: usize) -> Option<&Interpolated> {
        match &self.component_stack[index] {
            ChunkNoiseFunctionComponent::Chunk(
                ChunkSpecificNoiseFunctionComponent::Interpolated(interpolated),
            ) => Some(interpolated),
            _ => None,
        }
    }

    /// Assemble every interpolator's corner grid from cached columns, then
    /// evaluate and publish only the missing ones. Returns `false` when the
    /// grids cannot be shared: inputs that depend on per-chunk structures, or
    /// interpolators on different corner grids.
    fn prepare_shared_columns(
        &mut self,
        volume: &DensityVolume,
        columns: &super::corner_cache::CornerColumnCache,
    ) -> bool {
        use super::{corner_cache::ColumnKey, density_volume::DensityBuffer};
        let beardifier = self
            .component_stack
            .iter()
            .position(|component| {
                matches!(
                    component,
                    ChunkNoiseFunctionComponent::Chunk(
                        ChunkSpecificNoiseFunctionComponent::Beardifier(_)
                    )
                )
            })
            .unwrap_or(usize::MAX);
        let interpolators: Vec<usize> = (0..=self.final_density)
            .filter(|&index| self.interpolated(index).is_some())
            .collect();
        let Some(grid) = interpolators
            .first()
            .and_then(|&index| self.interpolated(index))
            .map(|interpolated| interpolated.corner_volume(volume).0)
        else {
            return false;
        };
        if !interpolators.iter().all(|&index| {
            self.interpolated(index).is_some_and(|interpolated| {
                interpolated.input_index < beardifier
                    && interpolated.corner_volume(volume).0 == grid
            })
        }) {
            return false;
        }
        let height = grid.size_y;
        let key = |x: usize, z: usize| ColumnKey {
            x: grid.block_x(x),
            z: grid.block_z(z),
            min_y: grid.min_block_y,
            size_y: height as u32,
        };
        let mut grids: Vec<DensityBuffer> = interpolators
            .iter()
            .map(|_| DensityBuffer::acquire(&grid))
            .collect();
        let mut missing = Vec::new();
        for z in 0..grid.size_z {
            for x in 0..grid.size_x {
                let start = grid.index_unchecked(x, 0, z);
                if let Some(values) = columns.get(key(x, z)) {
                    for (corners, values) in grids.iter_mut().zip(values.chunks_exact(height)) {
                        corners[start..start + height].copy_from_slice(values);
                    }
                } else {
                    missing.push((x, z));
                }
            }
        }
        let input = |stack: &[ChunkNoiseFunctionComponent], index: usize| match &stack[index] {
            ChunkNoiseFunctionComponent::Chunk(
                ChunkSpecificNoiseFunctionComponent::Interpolated(interpolated),
            ) => interpolated.input_index,
            _ => unreachable!("collected interpolators"),
        };
        if missing.len() == grid.size_x * grid.size_z {
            // Nothing shared yet: keep the single batched grid evaluation.
            for (corners, &index) in grids.iter_mut().zip(&interpolators) {
                let input = input(&self.component_stack, index);
                ChunkNoiseFunctionComponent::sample_volume_from_stack(
                    &mut self.component_stack[..=input],
                    corners,
                    &grid,
                );
            }
        } else {
            // Evaluate all interpolators per column so shared subgraph caches
            // keyed on the column volume are reused between them.
            let mut column = DensityBuffer::with_len(height);
            for &(x, z) in &missing {
                let column_volume = DensityVolume::new(
                    1,
                    height,
                    1,
                    grid.block_x(x),
                    grid.min_block_y,
                    grid.block_z(z),
                    grid.step_block_x,
                    grid.step_block_y,
                    grid.step_block_z,
                );
                let start = grid.index_unchecked(x, 0, z);
                for (corners, &index) in grids.iter_mut().zip(&interpolators) {
                    let input = input(&self.component_stack, index);
                    ChunkNoiseFunctionComponent::sample_volume_from_stack(
                        &mut self.component_stack[..=input],
                        &mut column,
                        &column_volume,
                    );
                    corners[start..start + height].copy_from_slice(&column);
                }
            }
        }
        for &(x, z) in &missing {
            let start = grid.index_unchecked(x, 0, z);
            let values: std::sync::Arc<[f32]> = grids
                .iter()
                .flat_map(|corners| corners[start..start + height].iter().copied())
                .collect();
            columns.insert(key(x, z), values);
        }
        for (corners, index) in grids.into_iter().zip(interpolators) {
            if let ChunkNoiseFunctionComponent::Chunk(
                ChunkSpecificNoiseFunctionComponent::Interpolated(interpolated),
            ) = &mut self.component_stack[index]
            {
                interpolated.set_prepared(grid, corners);
            }
        }
        true
    }

    /// Conservative bounds of `final_density` inside an inclusive block box.
    #[must_use]
    pub(crate) fn final_density_bounds(
        &self,
        area: &super::interval::BlockBounds,
    ) -> super::interval::Bounds {
        super::interval::bounds(&self.component_stack, self.final_density, area)
    }

    pub(crate) fn reuse_interpolation_corners(&mut self) {
        for component in &mut self.component_stack {
            if let ChunkNoiseFunctionComponent::Chunk(
                ChunkSpecificNoiseFunctionComponent::Interpolated(interpolated),
            ) = component
            {
                interpolated.reuse_corners();
            }
        }
    }

    #[must_use]
    pub fn component_stack_mut(&mut self) -> &mut [ChunkNoiseFunctionComponent<'a>] {
        &mut self.component_stack
    }

    pub(super) fn generate_stack(
        base: &'a [ProtoNoiseFunctionComponent],
        build_options: Option<&ChunkNoiseFunctionBuilderOptions>,
    ) -> Box<[ChunkNoiseFunctionComponent<'a>]> {
        let mut component_stack = Vec::<ChunkNoiseFunctionComponent>::with_capacity(base.len());

        for base_component in base {
            let chunk_component = match base_component {
                ProtoNoiseFunctionComponent::Dependent(dependent) => {
                    ChunkNoiseFunctionComponent::from_dependent(dependent)
                }
                ProtoNoiseFunctionComponent::Independent(independent) => {
                    ChunkNoiseFunctionComponent::Independent(independent)
                }
                ProtoNoiseFunctionComponent::PassThrough(pass_through) => {
                    ChunkNoiseFunctionComponent::PassThrough(pass_through.clone())
                }
                ProtoNoiseFunctionComponent::Beardifier(_) => match build_options {
                    Some(options) => ChunkNoiseFunctionComponent::Chunk(
                        ChunkSpecificNoiseFunctionComponent::Beardifier(Beardifier::new(
                            options.beardifier_structures.clone(),
                            options.beardifier_junctions.clone(),
                            options.affected_box,
                        )),
                    ),
                    None => ChunkNoiseFunctionComponent::Independent(&BEARDIFIER_ZERO_CONSTANT),
                },
                ProtoNoiseFunctionComponent::Wrapper(wrapper) => {
                    let min_value = component_stack[wrapper.input_index].min();
                    let max_value = component_stack[wrapper.input_index].max();

                    match wrapper.wrapper_type {
                        WrapperType::Interpolated { .. } if build_options.is_none() => {
                            ChunkNoiseFunctionComponent::PassThrough(PassThrough::new(
                                wrapper.input_index,
                                min_value,
                                max_value,
                            ))
                        }
                        WrapperType::Interpolated {
                            cell_size_xz,
                            cell_size_y,
                        } => ChunkNoiseFunctionComponent::Chunk(
                            ChunkSpecificNoiseFunctionComponent::Interpolated(Interpolated::new(
                                wrapper.input_index,
                                cell_size_xz,
                                cell_size_y,
                                min_value,
                                max_value,
                            )),
                        ),
                        WrapperType::Cache => ChunkNoiseFunctionComponent::Chunk(
                            ChunkSpecificNoiseFunctionComponent::Cache(Cache::new(
                                wrapper.input_index,
                                base[wrapper.input_index].column_y(),
                                min_value,
                                max_value,
                            )),
                        ),
                    }
                }
            };
            component_stack.push(chunk_component);
        }
        component_stack.into_boxed_slice()
    }

    #[must_use]
    pub fn generate(
        base: &'a ProtoNoiseRouter,
        build_options: &ChunkNoiseFunctionBuilderOptions,
    ) -> Self {
        Self {
            barrier_noise: base.barrier_noise,
            fluid_level_floodedness_noise: base.fluid_level_floodedness_noise,
            fluid_level_spread_noise: base.fluid_level_spread_noise,
            lava_noise: base.lava_noise,
            erosion: base.erosion,
            depth: base.depth,
            final_density: base.final_density,
            vein_toggle: base.vein_toggle,
            vein_ridged: base.vein_ridged,
            vein_gap: base.vein_gap,
            component_stack: Self::generate_stack(&base.full_component_stack, Some(build_options)),
        }
    }
}
