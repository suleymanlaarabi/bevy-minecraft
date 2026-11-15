use pumpkin_util::math::vector3::Vector3;

use crate::{
    biome::multi_noise::{NoiseValuePoint, to_long},
    generation::biome_coords,
};

use super::{
    chunk_noise_router::{ChunkNoiseFunctionComponent, ChunkNoiseRouter},
    density_volume::{DensityBuffer, DensityVolume},
    proto_noise_router::ProtoMultiNoiseRouter,
};

pub struct MultiNoiseSampler<'a> {
    temperature: usize,
    humidity: usize,
    continentalness: usize,
    erosion: usize,
    depth: usize,
    // AKA: Weirdness
    ridges: usize,
    component_stack: Box<[ChunkNoiseFunctionComponent<'a>]>,
    volume: Option<DensityVolume>,
    buffers: Option<[DensityBuffer; 6]>,
    column_y: [Option<i32>; 6],
}

impl<'a> MultiNoiseSampler<'a> {
    pub fn fill_volume(&mut self, volume: DensityVolume) {
        let indices = [
            self.temperature,
            self.humidity,
            self.continentalness,
            self.erosion,
            self.depth,
            self.ridges,
        ];
        let buffers = std::array::from_fn(|field| {
            // Five Overworld climate fields are planes; keep only their X/Z
            // samples instead of duplicating them throughout the chunk height.
            let input_volume = self.column_y[field].map_or(volume, |y| DensityVolume {
                size_y: 1,
                min_block_y: y,
                step_block_y: 1,
                ..volume
            });
            let mut buffer = DensityBuffer::acquire(&input_volume);
            ChunkNoiseFunctionComponent::sample_volume_from_stack(
                &mut self.component_stack[..=indices[field]],
                &mut buffer,
                &input_volume,
            );
            buffer
        });
        self.volume = Some(volume);
        self.buffers = Some(buffers);
    }

    pub fn sample(&mut self, biome_x: i32, biome_y: i32, biome_z: i32) -> NoiseValuePoint {
        let block_x = biome_coords::to_block(biome_x);
        let block_y = biome_coords::to_block(biome_y);
        let block_z = biome_coords::to_block(biome_z);

        if let (Some(volume), Some(buffers)) = (&self.volume, &self.buffers)
            && let Some(index) = volume.index_of_block(block_x, block_y, block_z)
        {
            let column = index / volume.size_y;
            let sample = |field: usize| {
                to_long(
                    buffers[field][if self.column_y[field].is_some() {
                        column
                    } else {
                        index
                    }],
                )
            };
            return NoiseValuePoint {
                temperature: sample(0),
                humidity: sample(1),
                continentalness: sample(2),
                erosion: sample(3),
                depth: sample(4),
                weirdness: sample(5),
            };
        }

        let pos = Vector3::new(block_x, block_y, block_z);

        let temperature = ChunkNoiseFunctionComponent::sample_from_stack(
            &mut self.component_stack[..=self.temperature],
            &pos,
        );

        let humidity = ChunkNoiseFunctionComponent::sample_from_stack(
            &mut self.component_stack[..=self.humidity],
            &pos,
        );

        let continentalness = ChunkNoiseFunctionComponent::sample_from_stack(
            &mut self.component_stack[..=self.continentalness],
            &pos,
        );

        let erosion = ChunkNoiseFunctionComponent::sample_from_stack(
            &mut self.component_stack[..=self.erosion],
            &pos,
        );

        let depth = ChunkNoiseFunctionComponent::sample_from_stack(
            &mut self.component_stack[..=self.depth],
            &pos,
        );

        let weirdness = ChunkNoiseFunctionComponent::sample_from_stack(
            &mut self.component_stack[..=self.ridges],
            &pos,
        );

        NoiseValuePoint {
            temperature: to_long(temperature),
            humidity: to_long(humidity),
            continentalness: to_long(continentalness),
            erosion: to_long(erosion),
            depth: to_long(depth),
            weirdness: to_long(weirdness),
        }
    }

    pub fn sample_erosion(&mut self, block_x: i32, block_y: i32, block_z: i32) -> f32 {
        let pos = Vector3::new(block_x, block_y, block_z);

        ChunkNoiseFunctionComponent::sample_from_stack(
            &mut self.component_stack[..=self.erosion],
            &pos,
        )
    }

    #[must_use]
    pub fn generate(base: &'a ProtoMultiNoiseRouter) -> Self {
        Self {
            temperature: base.temperature,
            humidity: base.vegetation,
            continentalness: base.continents,
            depth: base.depth,
            erosion: base.erosion,
            ridges: base.ridges,
            component_stack: ChunkNoiseRouter::generate_stack(&base.full_component_stack, None),
            volume: None,
            buffers: None,
            column_y: [
                base.temperature,
                base.vegetation,
                base.continents,
                base.erosion,
                base.depth,
                base.ridges,
            ]
            .map(|index| base.full_component_stack[index].column_y()),
        }
    }
}
