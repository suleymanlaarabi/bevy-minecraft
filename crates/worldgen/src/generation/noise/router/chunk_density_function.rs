use pumpkin_util::math::{block_box::BlockBox, lerp, vector3::Vector3};

use super::{
    chunk_noise_router::{ChunkNoiseFunctionComponent, MutableChunkNoiseFunctionComponentImpl},
    density_function::{
        NoiseFunctionComponentRange,
        beardifier::{Beardifier, BeardifierJunction, BeardifierStructure},
    },
    density_volume::{DensityBuffer, DensityVolume},
};

pub struct ChunkNoiseFunctionBuilderOptions {
    pub beardifier_structures: Vec<BeardifierStructure>,
    pub beardifier_junctions: Vec<BeardifierJunction>,
    pub affected_box: Option<BlockBox>,
}

impl ChunkNoiseFunctionBuilderOptions {
    #[must_use]
    pub const fn new(
        beardifier_structures: Vec<BeardifierStructure>,
        beardifier_junctions: Vec<BeardifierJunction>,
        affected_box: Option<BlockBox>,
    ) -> Self {
        Self {
            beardifier_structures,
            beardifier_junctions,
            affected_box,
        }
    }
}

pub struct Cache {
    pub(crate) input_index: usize,
    volume: Option<DensityVolume>,
    buffer: Option<DensityBuffer>,
    value_key: Option<Vector3<i32>>,
    column_y: Option<i32>,
    value: f32,
    min_value: f32,
    max_value: f32,
}

impl Cache {
    #[must_use]
    pub const fn new(
        input_index: usize,
        column_y: Option<i32>,
        min_value: f32,
        max_value: f32,
    ) -> Self {
        // Slice(Y) is independent of the requested Y. Canonicalize both point
        // keys and volume storage instead of reevaluating every vertical step.
        Self {
            input_index,
            volume: None,
            buffer: None,
            value_key: None,
            column_y,
            value: 0.0,
            min_value,
            max_value,
        }
    }
}

impl NoiseFunctionComponentRange for Cache {
    #[inline]
    fn min(&self) -> f32 {
        self.min_value
    }

    #[inline]
    fn max(&self) -> f32 {
        self.max_value
    }
}

impl MutableChunkNoiseFunctionComponentImpl for Cache {
    fn sample(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        pos: &Vector3<i32>,
    ) -> f32 {
        let pos = &Vector3::new(pos.x, self.column_y.unwrap_or(pos.y), pos.z);
        if self.value_key == Some(*pos) {
            return self.value;
        }
        if let (Some(volume), Some(buffer)) = (&self.volume, &self.buffer)
            && let Some(index) = volume.index_of_block(pos.x, pos.y, pos.z)
        {
            return buffer[index];
        }
        let value = ChunkNoiseFunctionComponent::sample_from_stack(
            &mut component_stack[..=self.input_index],
            pos,
        );
        self.value_key = Some(*pos);
        self.value = value;
        value
    }

    fn sample_volume(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        let input_volume = self.column_y.map_or(*volume, |y| DensityVolume {
            size_y: 1,
            min_block_y: y,
            step_block_y: 1,
            ..*volume
        });
        if self.volume.as_ref() != Some(&input_volume) || self.buffer.is_none() {
            self.buffer = None;
            let mut cached = DensityBuffer::acquire(&input_volume);
            ChunkNoiseFunctionComponent::sample_volume_from_stack(
                &mut component_stack[..=self.input_index],
                &mut cached,
                &input_volume,
            );
            self.volume = Some(input_volume);
            self.buffer = Some(cached);
        }
        if let Some(cached) = &self.buffer {
            if self.column_y.is_some() {
                for (column, value) in buffer.chunks_exact_mut(volume.size_y).zip(cached.iter()) {
                    column.fill(*value);
                }
            } else {
                buffer.copy_from_slice(cached);
            }
        }
    }
}

// Structure height probes revisit nearby columns. Retain only one chunk's
// corner grid per interpolator; missing columns use the existing volume kernel.
// No terrain columns are precomputed for isolated height requests.
struct CornerTile {
    volume: DensityVolume,
    values: Box<[f32]>,
    present: Box<[u64]>,
}

impl CornerTile {
    fn volume(request: &DensityVolume) -> DensityVolume {
        let axis = |block: i32, step: i32| {
            let start = block.div_euclid(16) * 16;
            let min = start.div_euclid(step) * step;
            let max = (start + 16).div_euclid(step) * step;
            let max = if max < start + 16 { max + step } else { max };
            (min, ((max - min) / step + 1) as usize)
        };
        let (x, size_x) = axis(request.min_block_x, request.step_block_x);
        let (z, size_z) = axis(request.min_block_z, request.step_block_z);
        DensityVolume::new(
            size_x,
            request.size_y,
            size_z,
            x,
            request.min_block_y,
            z,
            request.step_block_x,
            request.step_block_y,
            request.step_block_z,
        )
    }

    fn new(volume: DensityVolume) -> Self {
        Self {
            values: vec![0.0; volume.size()].into_boxed_slice(),
            present: vec![0; (volume.size_x * volume.size_z).div_ceil(64)].into_boxed_slice(),
            volume,
        }
    }

    fn sample(
        &mut self,
        stack: &mut [ChunkNoiseFunctionComponent],
        output: &mut [f32],
        request: &DensityVolume,
    ) {
        for z in 0..request.size_z {
            for x in 0..request.size_x {
                let block_x = request.block_x(x);
                let block_z = request.block_z(z);
                let cached = self
                    .volume
                    .index_of_block(block_x, request.min_block_y, block_z)
                    .expect("Height probe corners must fit their chunk tile");
                let column = cached / self.volume.size_y;
                let mask = 1u64 << (column % 64);
                let target = request.index_unchecked(x, 0, z);
                let output = &mut output[target..target + request.size_y];
                let values = &mut self.values[cached..cached + request.size_y];
                if self.present[column / 64] & mask == 0 {
                    let volume = DensityVolume::new(
                        1,
                        request.size_y,
                        1,
                        block_x,
                        request.min_block_y,
                        block_z,
                        request.step_block_x,
                        request.step_block_y,
                        request.step_block_z,
                    );
                    ChunkNoiseFunctionComponent::sample_volume_from_stack(stack, values, &volume);
                    self.present[column / 64] |= mask;
                }
                output.copy_from_slice(values);
            }
        }
    }
}

pub struct Interpolated {
    pub(crate) input_index: usize,
    cell_size_xz: i32,
    cell_size_y: i32,
    cell_size_xz_inv: f32,
    cell_size_y_inv: f32,
    min_value: f32,
    max_value: f32,
    reuse_corners: bool,
    corners: Option<Box<CornerTile>>,
    /// Corner grid of one chunk volume, evaluated once by [`Self::prepare`] and
    /// shared by every block sub-volume of that chunk.
    prepared: Option<(DensityVolume, DensityBuffer)>,
}

impl Interpolated {
    pub(crate) fn reuse_corners(&mut self) {
        self.reuse_corners = true;
    }

    pub(crate) fn coarsen(&mut self) {
        self.cell_size_xz *= 2;
        self.cell_size_xz_inv = 1.0 / self.cell_size_xz as f32;
        self.cell_size_y *= 2;
        self.cell_size_y_inv = 1.0 / self.cell_size_y as f32;
    }

    #[must_use]
    pub const fn new(
        input_index: usize,
        cell_size_xz: i32,
        cell_size_y: i32,
        min_value: f32,
        max_value: f32,
    ) -> Self {
        Self {
            input_index,
            cell_size_xz,
            cell_size_y,
            cell_size_xz_inv: 1.0 / cell_size_xz as f32,
            cell_size_y_inv: 1.0 / cell_size_y as f32,
            min_value,
            max_value,
            reuse_corners: false,
            corners: None,
            prepared: None,
        }
    }

    /// Evaluate the whole corner grid of `volume` in one batch. Later block
    /// requests inside `volume` interpolate from it instead of resampling.
    pub(crate) fn prepare(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        volume: &DensityVolume,
    ) {
        let (cell_volume, _) = self.corner_volume(volume);
        let mut corners = DensityBuffer::acquire(&cell_volume);
        ChunkNoiseFunctionComponent::sample_volume_from_stack(
            &mut component_stack[..=self.input_index],
            &mut corners,
            &cell_volume,
        );
        self.prepared = Some((cell_volume, corners));
    }

    /// Bounds of the interpolated value inside an inclusive block box, from
    /// the prepared corners of every cell overlapping it.
    pub(crate) fn corner_bounds(&self, area: &super::interval::BlockBounds) -> Option<(f32, f32)> {
        let (grid, corners) = self.prepared.as_ref()?;
        let axis = |min: i32, max: i32, origin: i32, step: i32, size: usize| {
            let first = (min - origin).div_euclid(step);
            let last = (max - origin + step - 1).div_euclid(step);
            (first >= 0 && (last as usize) < size).then_some(first as usize..=last as usize)
        };
        let xs = axis(
            area.min[0],
            area.max[0],
            grid.min_block_x,
            grid.step_block_x,
            grid.size_x,
        )?;
        let ys = axis(
            area.min[1],
            area.max[1],
            grid.min_block_y,
            grid.step_block_y,
            grid.size_y,
        )?;
        let zs = axis(
            area.min[2],
            area.max[2],
            grid.min_block_z,
            grid.step_block_z,
            grid.size_z,
        )?;
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for z in zs {
            for x in xs.clone() {
                let start = grid.index_unchecked(x, *ys.start(), z);
                for &value in &corners[start..=start + (ys.end() - ys.start())] {
                    lo = lo.min(value);
                    hi = hi.max(value);
                }
            }
        }
        // The nested f32 lerps may round a few ulps past the corner range.
        // Widen here, before discontinuous range choices consume the bounds.
        let slack = lo.abs().max(hi.abs()) * 1.0e-5 + 1.0e-6;
        Some((lo - slack, hi + slack))
    }

    /// Copy `cell_volume` corners from the prepared grid when it covers them.
    fn copy_prepared(&self, output: &mut [f32], cell_volume: &DensityVolume) -> bool {
        let Some((grid, corners)) = &self.prepared else {
            return false;
        };
        if (grid.step_block_x, grid.step_block_y, grid.step_block_z)
            != (
                cell_volume.step_block_x,
                cell_volume.step_block_y,
                cell_volume.step_block_z,
            )
            || grid
                .index_of_block(
                    cell_volume.max_block_x(),
                    cell_volume.max_block_y(),
                    cell_volume.max_block_z(),
                )
                .is_none()
        {
            return false;
        }
        let Some(origin) = grid.index_of_block(
            cell_volume.min_block_x,
            cell_volume.min_block_y,
            cell_volume.min_block_z,
        ) else {
            return false;
        };
        for z in 0..cell_volume.size_z {
            for x in 0..cell_volume.size_x {
                let source = origin + grid.index_unchecked(x, 0, z);
                let target = cell_volume.index_unchecked(x, 0, z);
                output[target..target + cell_volume.size_y]
                    .copy_from_slice(&corners[source..source + cell_volume.size_y]);
            }
        }
        true
    }

    /// Install a corner grid assembled outside [`Self::prepare`].
    pub(crate) fn set_prepared(&mut self, grid: DensityVolume, corners: DensityBuffer) {
        self.prepared = Some((grid, corners));
    }

    pub(crate) fn corner_volume(&self, volume: &DensityVolume) -> (DensityVolume, [usize; 3]) {
        let min_cell_x = volume.min_block_x.div_euclid(self.cell_size_xz);
        let min_cell_y = volume.min_block_y.div_euclid(self.cell_size_y);
        let min_cell_z = volume.min_block_z.div_euclid(self.cell_size_xz);
        let max_block_x = volume.max_block_x();
        let max_block_y = volume.max_block_y();
        let max_block_z = volume.max_block_z();
        let cell_count_x = (max_block_x.div_euclid(self.cell_size_xz) - min_cell_x + 1) as usize;
        let cell_count_y = (max_block_y.div_euclid(self.cell_size_y) - min_cell_y + 1) as usize;
        let cell_count_z = (max_block_z.div_euclid(self.cell_size_xz) - min_cell_z + 1) as usize;
        let corner_count = |count: usize, max_block: i32, cell_size: i32| {
            if max_block.rem_euclid(cell_size) == 0 {
                count
            } else {
                count + 1
            }
        };
        let cell_volume = DensityVolume::new(
            corner_count(cell_count_x, max_block_x, self.cell_size_xz),
            corner_count(cell_count_y, max_block_y, self.cell_size_y),
            corner_count(cell_count_z, max_block_z, self.cell_size_xz),
            min_cell_x * self.cell_size_xz,
            min_cell_y * self.cell_size_y,
            min_cell_z * self.cell_size_xz,
            self.cell_size_xz,
            self.cell_size_y,
            self.cell_size_xz,
        );
        (cell_volume, [cell_count_x, cell_count_y, cell_count_z])
    }

    const fn is_cell_aligned(&self, volume: &DensityVolume) -> bool {
        (volume.step_block_x == self.cell_size_xz || volume.size_x == 1)
            && (volume.step_block_y == self.cell_size_y || volume.size_y == 1)
            && (volume.step_block_z == self.cell_size_xz || volume.size_z == 1)
            && volume.min_block_x.rem_euclid(self.cell_size_xz) == 0
            && volume.min_block_y.rem_euclid(self.cell_size_y) == 0
            && volume.min_block_z.rem_euclid(self.cell_size_xz) == 0
    }

    fn sample_with_block_step(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        let (cell_volume, [cell_count_x, cell_count_y, cell_count_z]) = self.corner_volume(volume);
        let mut corners = DensityBuffer::acquire(&cell_volume);
        // Nested interpolation can request a wider volume. Cache only grids
        // fitting one tile, with at most 32 KiB of f32 data per interpolator.
        let tile = self
            .reuse_corners
            .then(|| CornerTile::volume(&cell_volume))
            .filter(|tile| {
                tile.size() <= 8192
                    && tile
                        .index_of_block(
                            cell_volume.block_x(cell_volume.size_x - 1),
                            cell_volume.min_block_y,
                            cell_volume.block_z(cell_volume.size_z - 1),
                        )
                        .is_some()
            });
        if self.copy_prepared(&mut corners, &cell_volume) {
        } else if let Some(tile) = tile {
            if self
                .corners
                .as_ref()
                .is_none_or(|cache| cache.volume != tile)
            {
                self.corners = Some(Box::new(CornerTile::new(tile)));
            }
            self.corners.as_mut().unwrap().sample(
                &mut component_stack[..=self.input_index],
                &mut corners,
                &cell_volume,
            );
        } else {
            ChunkNoiseFunctionComponent::sample_volume_from_stack(
                &mut component_stack[..=self.input_index],
                &mut corners,
                &cell_volume,
            );
        }

        for cell_z in 0..cell_count_z {
            let next_z = (cell_z + 1).min(cell_volume.size_z - 1);
            for cell_x in 0..cell_count_x {
                let next_x = (cell_x + 1).min(cell_volume.size_x - 1);
                for cell_y in 0..cell_count_y {
                    let next_y = (cell_y + 1).min(cell_volume.size_y - 1);
                    let corner = |x: usize, y: usize, z: usize| {
                        corners[cell_volume.index_unchecked(x, y, z)]
                    };
                    let values = [
                        corner(cell_x, cell_y, cell_z),
                        corner(next_x, cell_y, cell_z),
                        corner(cell_x, next_y, cell_z),
                        corner(next_x, next_y, cell_z),
                        corner(cell_x, cell_y, next_z),
                        corner(next_x, cell_y, next_z),
                        corner(cell_x, next_y, next_z),
                        corner(next_x, next_y, next_z),
                    ];
                    if !volume.is_block_step() {
                        self.fill_sparse_cell(
                            buffer,
                            volume,
                            &cell_volume,
                            [cell_x, cell_y, cell_z],
                            values,
                        );
                        continue;
                    }
                    self.fill_cell(
                        buffer,
                        volume,
                        &cell_volume,
                        [cell_x, cell_y, cell_z],
                        values,
                    );
                }
            }
        }
    }

    fn fill_cell(
        &self,
        buffer: &mut [f32],
        volume: &DensityVolume,
        cell_volume: &DensityVolume,
        cell: [usize; 3],
        [v000, v100, v010, v110, v001, v101, v011, v111]: [f32; 8],
    ) {
        let cell_out_x = cell_volume.block_x(cell[0]) - volume.min_block_x;
        let cell_out_y = cell_volume.block_y(cell[1]) - volume.min_block_y;
        let cell_out_z = cell_volume.block_z(cell[2]) - volume.min_block_z;
        let x0 = 0.max(-cell_out_x);
        let y0 = 0.max(-cell_out_y);
        let z0 = 0.max(-cell_out_z);
        let x1 = self.cell_size_xz.min(volume.size_x as i32 - cell_out_x) - 1;
        let y1 = self.cell_size_y.min(volume.size_y as i32 - cell_out_y) - 1;
        let z1 = self.cell_size_xz.min(volume.size_z as i32 - cell_out_z) - 1;

        for z in z0..=z1 {
            let alpha_z = z as f32 * self.cell_size_xz_inv;
            let out_z = (cell_out_z + z) as usize;
            let v00 = lerp(alpha_z, v000, v001);
            let v01 = lerp(alpha_z, v010, v011);
            let v10 = lerp(alpha_z, v100, v101);
            let v11 = lerp(alpha_z, v110, v111);
            for x in x0..=x1 {
                let alpha_x = x as f32 * self.cell_size_xz_inv;
                let out_x = (cell_out_x + x) as usize;
                let v_0 = lerp(alpha_x, v00, v10);
                let v_1 = lerp(alpha_x, v01, v11);
                let value_step = (v_1 - v_0) * self.cell_size_y_inv;
                let start = volume.index_unchecked(out_x, (cell_out_y + y0) as usize, out_z);
                // Independent lanes (no loop-carried sum) let the column vectorize.
                for (y, slot) in (y0..).zip(&mut buffer[start..start + (y1 - y0 + 1) as usize]) {
                    *slot = v_0 + value_step * y as f32;
                }
            }
        }
    }

    // Match the dense kernel's Y rounding, while only computing requested
    // X/Z columns and never allocating a dense intermediate volume.
    fn fill_sparse_cell(
        &self,
        buffer: &mut [f32],
        volume: &DensityVolume,
        corners: &DensityVolume,
        cell: [usize; 3],
        [v000, v100, v010, v110, v001, v101, v011, v111]: [f32; 8],
    ) {
        let ox = corners.block_x(cell[0]) - volume.min_block_x;
        let oy = corners.block_y(cell[1]) - volume.min_block_y;
        let oz = corners.block_z(cell[2]) - volume.min_block_z;
        let range = |offset: i32, width: i32, step: i32, size: usize| {
            let first = (offset.max(0) as usize).div_ceil(step as usize);
            let end = ((offset + width).max(0) as usize).div_ceil(step as usize);
            first.min(size)..end.min(size)
        };
        let ys = range(oy, self.cell_size_y, volume.step_block_y, volume.size_y);
        if ys.is_empty() {
            return;
        }
        for z in range(oz, self.cell_size_xz, volume.step_block_z, volume.size_z) {
            let az = (z as i32 * volume.step_block_z - oz) as f32 * self.cell_size_xz_inv;
            let v00 = lerp(az, v000, v001);
            let v01 = lerp(az, v010, v011);
            let v10 = lerp(az, v100, v101);
            let v11 = lerp(az, v110, v111);
            for x in range(ox, self.cell_size_xz, volume.step_block_x, volume.size_x) {
                let ax = (x as i32 * volume.step_block_x - ox) as f32 * self.cell_size_xz_inv;
                let low = lerp(ax, v00, v10);
                let high = lerp(ax, v01, v11);
                let step = (high - low) * self.cell_size_y_inv;
                for y in ys.clone() {
                    let in_cell = y as i32 * volume.step_block_y - oy;
                    buffer[volume.index_unchecked(x, y, z)] = low + step * in_cell as f32;
                }
            }
        }
    }
}

impl NoiseFunctionComponentRange for Interpolated {
    #[inline]
    fn min(&self) -> f32 {
        self.min_value
    }

    #[inline]
    fn max(&self) -> f32 {
        self.max_value
    }
}

impl MutableChunkNoiseFunctionComponentImpl for Interpolated {
    fn sample(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        pos: &Vector3<i32>,
    ) -> f32 {
        let x_in_cell = pos.x.rem_euclid(self.cell_size_xz);
        let y_in_cell = pos.y.rem_euclid(self.cell_size_y);
        let z_in_cell = pos.z.rem_euclid(self.cell_size_xz);
        if x_in_cell == 0 && y_in_cell == 0 && z_in_cell == 0 {
            return ChunkNoiseFunctionComponent::sample_from_stack(
                &mut component_stack[..=self.input_index],
                pos,
            );
        }
        let cell_volume = DensityVolume::new(
            2,
            2,
            2,
            pos.x - x_in_cell,
            pos.y - y_in_cell,
            pos.z - z_in_cell,
            self.cell_size_xz,
            self.cell_size_y,
            self.cell_size_xz,
        );
        let mut corners = DensityBuffer::acquire(&cell_volume);
        ChunkNoiseFunctionComponent::sample_volume_from_stack(
            &mut component_stack[..=self.input_index],
            &mut corners,
            &cell_volume,
        );
        let corner = |x: usize, y: usize, z: usize| corners[cell_volume.index_unchecked(x, y, z)];
        let delta_x = x_in_cell as f32 / self.cell_size_xz as f32;
        let delta_y = y_in_cell as f32 / self.cell_size_y as f32;
        let delta_z = z_in_cell as f32 / self.cell_size_xz as f32;
        lerp(
            delta_z,
            lerp(
                delta_y,
                lerp(delta_x, corner(0, 0, 0), corner(1, 0, 0)),
                lerp(delta_x, corner(0, 1, 0), corner(1, 1, 0)),
            ),
            lerp(
                delta_y,
                lerp(delta_x, corner(0, 0, 1), corner(1, 0, 1)),
                lerp(delta_x, corner(0, 1, 1), corner(1, 1, 1)),
            ),
        )
    }

    fn sample_volume(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        buffer: &mut [f32],
        volume: &DensityVolume,
    ) {
        if self.is_cell_aligned(volume) {
            ChunkNoiseFunctionComponent::sample_volume_from_stack(
                &mut component_stack[..=self.input_index],
                buffer,
                volume,
            );
        } else {
            self.sample_with_block_step(component_stack, buffer, volume);
        }
    }
}

pub enum ChunkSpecificNoiseFunctionComponent {
    Cache(Cache),
    Interpolated(Interpolated),
    Beardifier(Beardifier),
}

impl NoiseFunctionComponentRange for ChunkSpecificNoiseFunctionComponent {
    #[inline]
    fn min(&self) -> f32 {
        match self {
            Self::Cache(c) => c.min(),
            Self::Interpolated(i) => i.min(),
            Self::Beardifier(b) => b.min(),
        }
    }

    #[inline]
    fn max(&self) -> f32 {
        match self {
            Self::Cache(c) => c.max(),
            Self::Interpolated(i) => i.max(),
            Self::Beardifier(b) => b.max(),
        }
    }
}

impl MutableChunkNoiseFunctionComponentImpl for ChunkSpecificNoiseFunctionComponent {
    #[inline]
    fn sample(
        &mut self,
        component_stack: &mut [ChunkNoiseFunctionComponent],
        pos: &Vector3<i32>,
    ) -> f32 {
        match self {
            Self::Cache(c) => c.sample(component_stack, pos),
            Self::Interpolated(i) => i.sample(component_stack, pos),
            Self::Beardifier(b) => b.sample(component_stack, pos),
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
            Self::Cache(c) => c.sample_volume(component_stack, buffer, volume),
            Self::Interpolated(i) => i.sample_volume(component_stack, buffer, volume),
            Self::Beardifier(b) => b.sample_volume(component_stack, buffer, volume),
        }
    }
}
