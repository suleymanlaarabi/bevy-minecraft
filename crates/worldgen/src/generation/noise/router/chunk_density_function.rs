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
    value: f32,
    min_value: f32,
    max_value: f32,
}

impl Cache {
    #[must_use]
    pub const fn new(input_index: usize, min_value: f32, max_value: f32) -> Self {
        Self {
            input_index,
            volume: None,
            buffer: None,
            value_key: None,
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
        if self.volume.as_ref() != Some(volume) || self.buffer.is_none() {
            self.buffer = None;
            let mut cached = DensityBuffer::acquire(volume);
            ChunkNoiseFunctionComponent::sample_volume_from_stack(
                &mut component_stack[..=self.input_index],
                &mut cached,
                volume,
            );
            self.volume = Some(*volume);
            self.buffer = Some(cached);
        }
        if let Some(cached) = &self.buffer {
            buffer.copy_from_slice(cached);
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
        }
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
        if let Some(tile) = tile {
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
                let mut value = v_0 + value_step * y0 as f32;
                let start = volume.index_unchecked(out_x, (cell_out_y + y0) as usize, out_z);
                for slot in &mut buffer[start..start + (y1 - y0 + 1) as usize] {
                    *slot = value;
                    value += value_step;
                }
            }
        }
    }

    // Match the dense kernel's incremental Y rounding, while only computing
    // requested X/Z columns and never allocating a dense intermediate volume.
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
                let first_y = 0.max(-oy);
                let mut value = low + step * first_y as f32;
                let last_y = (ys.end as i32 - 1) * volume.step_block_y - oy;
                for y in first_y..=last_y {
                    let relative = oy + y;
                    if relative % volume.step_block_y == 0 {
                        buffer[volume.index_unchecked(
                            x,
                            (relative / volume.step_block_y) as usize,
                            z,
                        )] = value;
                    }
                    value += step;
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
