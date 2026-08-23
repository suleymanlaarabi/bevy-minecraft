use super::{ChunkVoxels, VoxelKind, VoxelSettings};
use bevy::prelude::*;
use noiz::prelude::*;

pub(crate) fn generate_chunk(chunk: IVec2, settings: &VoxelSettings) -> ChunkVoxels {
    let origin = chunk * settings.chunk_size;
    let mut continental = Noise::<common_noise::Perlin>::default();
    continental.set_seed(settings.seed);
    continental.set_frequency(0.025);
    let mut hills = Noise::<common_noise::Perlin>::default();
    hills.set_seed(settings.seed.wrapping_add(1));
    hills.set_frequency(0.08);

    let mut details = Noise::<common_noise::Perlin>::default();
    details.set_seed(settings.seed.wrapping_add(2));
    details.set_frequency(0.20);

    let height_at = |world: IVec2| {
        let world = world.as_vec2();
        let continental: f32 = continental.sample(world);
        let hills: f32 = hills.sample(world);
        let details: f32 = details.sample(world);
        let height = settings.base_height + continental * 7.0 + hills * 3.0 + details;
        height.round().clamp(1.0, settings.max_height as f32) as i32
    };
    let size = settings.chunk_size;
    let mut cells = vec![VoxelKind::Air; (size * settings.max_height * size) as usize];
    for x in 0..size {
        for z in 0..size {
            let height = height_at(origin + IVec2::new(x, z));
            for y in 0..height {
                cells[(x + size * (z + size * y)) as usize] = VoxelKind::terrain_at(y, height);
            }
        }
    }

    let mut halo = Vec::with_capacity((size * 4) as usize);
    for side in 0..4 {
        halo.extend((0..size).map(|offset| {
            let local = match side {
                0 => IVec2::new(-1, offset),
                1 => IVec2::new(size, offset),
                2 => IVec2::new(offset, -1),
                _ => IVec2::new(offset, size),
            };
            height_at(origin + local)
        }));
    }
    ChunkVoxels::generated(size, settings.max_height, cells, halo)
}
