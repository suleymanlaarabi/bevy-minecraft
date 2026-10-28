use pumpkin_data::{Block, BlockId, BlockState, block_properties::blocks_movement};
use pumpkin_util::math::vector3::Vector3;
use rustc_hash::FxHashMap;

use crate::generation::{
    generator::VanillaGenerator,
    noise::{
        ChunkNoiseGenerator,
        aquifer_sampler::FluidLevel,
        router::{
            density_volume::DensityVolume,
            surface_height_sampler::{
                SurfaceHeightEstimateSampler, SurfaceHeightSamplerBuilderOptions,
            },
        },
    },
    proto_chunk::StandardChunkFluidLevelSampler,
};

use super::structures::HeightSampler;

pub struct NoiseHeightSampler<'a> {
    generator: &'a VanillaGenerator,
    preliminary: SurfaceHeightEstimateSampler<'a>,
    heights: FxHashMap<(i32, i32), i32>,
    ocean_floor_heights: FxHashMap<(i32, i32), i32>,
    columns: FxHashMap<(i32, i32), (i32, Vec<&'static BlockState>)>,
    height_noise: Option<ChunkNoiseGenerator<'a>>,
}

impl<'a> NoiseHeightSampler<'a> {
    pub fn new(generator: &'a VanillaGenerator) -> Self {
        let shape = &generator.settings.shape;
        let preliminary = SurfaceHeightEstimateSampler::generate(
            &generator.base_router.surface_estimator,
            &SurfaceHeightSamplerBuilderOptions::new(
                i32::from(shape.min_y),
                i32::from(shape.max_y()),
                shape.vertical_cell_block_count() as usize,
            ),
        );
        Self {
            generator,
            preliminary,
            heights: FxHashMap::default(),
            ocean_floor_heights: FxHashMap::default(),
            columns: FxHashMap::default(),
            height_noise: None,
        }
    }

    fn ensure_column_sampled(&mut self, x: i32, z: i32) {
        if self.columns.contains_key(&(x, z)) {
            return;
        }

        let (volume, mut noise) = self.noise_column(x, z);

        let densities = noise.sample_density();
        let mut blocks = Vec::with_capacity(volume.size_y);
        for y in 0..volume.size_y {
            let block_y = volume.block_y(y);
            let index = volume.index_unchecked(0, y, 0);
            let state = noise
                .sample_block_state(
                    &self.generator.random_config.ore_random_deriver,
                    &Vector3::new(x, block_y, z),
                    densities.density[index],
                    densities.vein_sample(index).as_ref(),
                    &mut self.preliminary,
                )
                .unwrap_or(self.generator.default_block);
            blocks.push(state);
        }
        self.columns.insert((x, z), (volume.min_block_y, blocks));
    }

    fn noise_column(&self, x: i32, z: i32) -> (DensityVolume, ChunkNoiseGenerator<'a>) {
        let generator = self.generator;
        let settings = generator.settings;
        let shape = &settings.shape;
        let fluid_sampler = StandardChunkFluidLevelSampler::new(
            FluidLevel::new(
                settings.sea_level,
                Block::from_state_id(settings.default_fluid.id),
            ),
            FluidLevel::new(-54, &Block::LAVA),
        );
        let volume = DensityVolume::with_block_step(
            1,
            shape.height as usize,
            1,
            x,
            i32::from(shape.min_y),
            z,
        );
        let mut noise = ChunkNoiseGenerator::new(
            &generator.base_router.noise,
            &generator.random_config,
            volume,
            shape,
            fluid_sampler,
            settings.aquifers_enabled,
            false,
            Vec::new(),
            Vec::new(),
            None,
        );
        if generator.fast_terrain {
            noise.router.coarsen_interpolation();
        }

        (volume, noise)
    }

    fn sample_column(&mut self, x: i32, z: i32, ocean_floor: bool) -> i32 {
        static FULL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *FULL.get_or_init(|| std::env::var("WORLDGEN_HEIGHT_BACKEND").as_deref() == Ok("full"))
            || self.columns.contains_key(&(x, z))
        {
            return self.full_column_height(x, z, ocean_floor);
        }
        let generator = self.generator;
        let [surface, floor] = generator
            .structure_heights
            .get_or_compute((x, z, generator.fast_terrain), || {
                self.compute_heights(x, z)
            });
        self.heights.insert((x, z), surface);
        self.ocean_floor_heights.insert((x, z), floor - 1);
        if ocean_floor { floor } else { surface }
    }

    fn compute_heights(&mut self, x: i32, z: i32) -> [i32; 2] {
        // Density interpolation is kept identical to the full-height volume.
        // Only block/aquifer evaluation stops once both height predicates match.
        let mut noise = if self.generator.fast_terrain {
            match self.height_noise.take() {
                Some(mut noise)
                    if noise.volume().min_block_x.div_euclid(16) == x.div_euclid(16)
                        && noise.volume().min_block_z.div_euclid(16) == z.div_euclid(16) =>
                {
                    noise.move_height_column(x, z);
                    noise
                }
                _ => {
                    let (_, mut noise) = self.noise_column(x, z);
                    noise.router.reuse_interpolation_corners();
                    noise
                }
            }
        } else {
            self.noise_column(x, z).1
        };
        let volume = *noise.volume();
        let densities = noise.sample_density();
        let mut surface = None;
        let mut floor = None;
        for y in (0..volume.size_y).rev() {
            let block_y = volume.block_y(y);
            let state = noise
                .sample_block_state(
                    &self.generator.random_config.ore_random_deriver,
                    &Vector3::new(x, block_y, z),
                    densities.density[y],
                    densities.vein_sample(y).as_ref(),
                    &mut self.preliminary,
                )
                .unwrap_or(self.generator.default_block);
            if surface.is_none() && !state.is_air() {
                surface = Some(block_y + 1);
            }
            if floor.is_none() && blocks_movement(state, BlockId::from_state_id(state.id)) {
                floor = Some(block_y + 1);
            }
            if surface.is_some() && floor.is_some() {
                break;
            }
        }
        let surface = surface.unwrap_or(volume.min_block_y);
        let floor = floor.unwrap_or(volume.min_block_y);
        if self.generator.fast_terrain {
            self.height_noise = Some(noise);
        }
        static VERIFY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *VERIFY.get_or_init(|| std::env::var("WORLDGEN_VERIFY_HEIGHTS").as_deref() == Ok("1")) {
            assert_eq!(
                surface,
                self.full_column_height(x, z, false),
                "surface at {x},{z}"
            );
            assert_eq!(
                floor,
                self.full_column_height(x, z, true),
                "ocean floor at {x},{z}"
            );
        }
        [surface, floor]
    }

    fn full_column_height(&mut self, x: i32, z: i32, ocean_floor: bool) -> i32 {
        self.ensure_column_sampled(x, z);
        let Some((min_y, blocks)) = self.columns.get(&(x, z)) else {
            return 0;
        };
        let min_y = *min_y;
        for (idx, &state) in blocks.iter().enumerate().rev() {
            let block_y = min_y + idx as i32;
            if if ocean_floor {
                blocks_movement(state, BlockId::from_state_id(state.id))
            } else {
                !state.is_air()
            } {
                return block_y + 1;
            }
        }

        min_y
    }
}

impl HeightSampler for NoiseHeightSampler<'_> {
    fn estimate_height(&mut self, block_x: i32, block_z: i32) -> i32 {
        let key = (block_x, block_z);
        if let Some(height) = self.heights.get(&key) {
            return *height;
        }
        let height = self.sample_column(block_x, block_z, false);
        self.heights.insert(key, height);
        height
    }

    fn estimate_ocean_floor_height(&mut self, block_x: i32, block_z: i32) -> i32 {
        let key = (block_x, block_z);
        if let Some(height) = self.ocean_floor_heights.get(&key) {
            return *height;
        }
        // Vanilla's structure helper asks for the highest occupied block, while
        // heightmaps store the first free block above it.
        let height = self.sample_column(block_x, block_z, true) - 1;
        self.ocean_floor_heights.insert(key, height);
        height
    }

    fn sample_column_block(
        &mut self,
        block_x: i32,
        block_z: i32,
        y: i32,
    ) -> Option<&'static BlockState> {
        self.ensure_column_sampled(block_x, block_z);
        let (min_y, blocks) = self.columns.get(&(block_x, block_z))?;
        let idx = y - *min_y;
        if idx >= 0 && (idx as usize) < blocks.len() {
            Some(blocks[idx as usize])
        } else {
            Some(Block::AIR.default_state)
        }
    }
}
