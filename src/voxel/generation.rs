use super::{ChunkVoxels, VoxelKind, VoxelSettings};
use bevy::prelude::*;
use noiz::{prelude::*, rng::NoiseRng};

type Simplex = Noise<common_noise::Simplex>;
const SAMPLE_MARGIN: i32 = 4;
const TREE_GRID: i32 = 8;
const TERRAIN_HEADROOM: i32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Biome {
    Plains,
    Forest,
    Desert,
    Alpine,
}

#[derive(Clone, Copy)]
struct Column {
    height: i32,
    biome: Biome,
    snow: bool,
}

#[derive(Resource)]
pub(crate) struct WorldGenerator {
    continental: Simplex,
    erosion: Simplex,
    peaks: Simplex,
    detail: Simplex,
    temperature: Simplex,
    humidity: Simplex,
    trees: NoiseRng,
}

impl WorldGenerator {
    pub(crate) fn new(seed: u32) -> Self {
        Self {
            continental: simplex(seed, 0.0025),
            erosion: simplex(seed.wrapping_add(1), 0.006),
            peaks: simplex(seed.wrapping_add(2), 0.012),
            detail: simplex(seed.wrapping_add(3), 0.055),
            temperature: simplex(seed.wrapping_add(4), 0.0018),
            humidity: simplex(seed.wrapping_add(5), 0.0022),
            trees: NoiseRng(seed.wrapping_add(6)),
        }
    }

    fn column(&self, world: IVec2, settings: &VoxelSettings) -> Column {
        let point = world.as_vec2();
        let continental: f32 = self.continental.sample(point);
        let erosion: f32 = self.erosion.sample(point);
        let peaks: f32 = self.peaks.sample(point);
        let detail: f32 = self.detail.sample(point);
        let temperature: f32 = self.temperature.sample(point);
        let humidity: f32 = self.humidity.sample(point);
        let sea = sea_level(settings);
        let ceiling = (settings.max_height - TERRAIN_HEADROOM).max(1) as f32;
        let room = (ceiling - sea as f32).max(1.0);
        let land = smoothstep(-0.32, 0.12, continental);
        let mountain_region =
            smoothstep(-0.10, 0.45, continental) * smoothstep(-0.25, 0.50, -erosion);
        let mountain_shape = smoothstep(-0.15, 0.65, peaks).powi(2);
        let base =
            (sea as f32 - 5.0 + land * 10.0 + erosion * 2.0 + detail * 1.25).clamp(1.0, ceiling);
        let height = base + mountain_region * mountain_shape * room * 0.58;
        let height = height.round().clamp(1.0, ceiling) as i32;
        let elevation = ((height - sea).max(0) as f32 / room).clamp(0.0, 1.0);
        let cold = temperature - elevation * 0.8;
        let biome = if cold < -0.20 && elevation > 0.45 {
            Biome::Alpine
        } else if temperature > 0.18 && humidity < -0.08 {
            Biome::Desert
        } else if humidity > -0.02 {
            Biome::Forest
        } else {
            Biome::Plains
        };
        let snow_line = sea as f32 + room * (0.62 + temperature * 0.10 + detail * 0.035);
        let snow = biome == Biome::Alpine && cold < -0.15 && height as f32 >= snow_line;
        Column {
            height,
            biome,
            snow,
        }
    }
}

pub(crate) fn generate_chunk(
    chunk: IVec2,
    settings: &VoxelSettings,
    generator: &WorldGenerator,
) -> ChunkVoxels {
    let size = settings.chunk_size;
    let height = settings.max_height;
    let origin = chunk * size;
    let sample_width = size + SAMPLE_MARGIN * 2;
    let mut columns = Vec::with_capacity((sample_width * sample_width) as usize);
    for z in -SAMPLE_MARGIN..size + SAMPLE_MARGIN {
        for x in -SAMPLE_MARGIN..size + SAMPLE_MARGIN {
            columns.push(generator.column(origin + IVec2::new(x, z), settings));
        }
    }
    let column_at = |x: i32, z: i32| {
        columns[((x + SAMPLE_MARGIN) + sample_width * (z + SAMPLE_MARGIN)) as usize]
    };

    let padded = size + 2;
    let mut area = vec![VoxelKind::Air; (padded * height * padded) as usize];
    let sea = sea_level(settings);
    for z in -1..=size {
        for x in -1..=size {
            let column = column_at(x, z);
            let slope = [
                column_at(x - 1, z).height,
                column_at(x + 1, z).height,
                column_at(x, z - 1).height,
                column_at(x, z + 1).height,
            ]
            .into_iter()
            .map(|neighbor| (column.height - neighbor).abs())
            .max()
            .unwrap_or(0);
            let surface = surface_kind(column, slope, sea);
            for y in 0..height {
                let kind = if y < column.height {
                    ground_kind(y, column.height, surface)
                } else if y < sea {
                    VoxelKind::Water
                } else {
                    VoxelKind::Air
                };
                area[area_index(x, y, z, padded)] = kind;
            }
        }
    }

    place_trees(&mut area, origin, size, height, padded, settings, generator);
    let mut cells = Vec::with_capacity((size * height * size) as usize);
    let mut halo = Vec::with_capacity((size * height * 4) as usize);
    for y in 0..height {
        for z in 0..size {
            for x in 0..size {
                cells.push(area[area_index(x, y, z, padded)]);
            }
        }
        for side in 0..4 {
            for offset in 0..size {
                let (x, z) = match side {
                    0 => (-1, offset),
                    1 => (size, offset),
                    2 => (offset, -1),
                    _ => (offset, size),
                };
                halo.push(area[area_index(x, y, z, padded)]);
            }
        }
    }
    ChunkVoxels::generated(size, height, cells, halo)
}

#[allow(clippy::too_many_arguments)]
fn place_trees(
    area: &mut [VoxelKind],
    origin: IVec2,
    size: i32,
    height: i32,
    padded: i32,
    settings: &VoxelSettings,
    generator: &WorldGenerator,
) {
    let min = origin - IVec2::splat(3);
    let max = origin + IVec2::splat(size + 2);
    for grid_z in min.y.div_euclid(TREE_GRID)..=max.y.div_euclid(TREE_GRID) {
        for grid_x in min.x.div_euclid(TREE_GRID)..=max.x.div_euclid(TREE_GRID) {
            let grid = IVec2::new(grid_x, grid_z);
            let position_bits = generator.trees.rand_u32(IVec3::new(grid_x, 0, grid_z));
            let world = grid * TREE_GRID
                + IVec2::new(
                    (position_bits & 7) as i32,
                    ((position_bits >> 3) & 7) as i32,
                );
            let local = world - origin;
            if local.x < -3 || local.x > size + 2 || local.y < -3 || local.y > size + 2 {
                continue;
            }
            let column = generator.column(world, settings);
            let chance =
                generator.trees.rand_u32(IVec3::new(grid_x, 1, grid_z)) as f32 / u32::MAX as f32;
            let density = match column.biome {
                Biome::Forest => 0.55,
                Biome::Plains => 0.08,
                _ => 0.0,
            };
            if chance >= density || column.height >= height - 7 {
                continue;
            }
            let slope = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y]
                .into_iter()
                .map(|offset| {
                    (column.height - generator.column(world + offset, settings).height).abs()
                })
                .max()
                .unwrap_or(0);
            if slope > 1 || surface_kind(column, slope, sea_level(settings)) != VoxelKind::Grass {
                continue;
            }
            let trunk = 4 + (generator.trees.rand_u32(IVec3::new(grid_x, 2, grid_z)) % 3) as i32;
            place_tree(area, local, column.height, trunk, height, padded);
        }
    }
}

fn place_tree(
    area: &mut [VoxelKind],
    root: IVec2,
    ground: i32,
    trunk: i32,
    height: i32,
    padded: i32,
) {
    for y in ground..ground + trunk {
        set_if_inside(
            area,
            root.x,
            y,
            root.y,
            height,
            padded,
            VoxelKind::Wood,
            true,
        );
    }
    let top = ground + trunk;
    for y in top - 2..=top + 1 {
        let radius: i32 = if y == top + 1 { 1 } else { 2 };
        for z in -radius..=radius {
            for x in -radius..=radius {
                if x.abs() + z.abs() <= radius * 2 {
                    set_if_inside(
                        area,
                        root.x + x,
                        y,
                        root.y + z,
                        height,
                        padded,
                        VoxelKind::Leaves,
                        false,
                    );
                }
            }
        }
    }
    set_if_inside(
        area,
        root.x,
        top,
        root.y,
        height,
        padded,
        VoxelKind::Wood,
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn set_if_inside(
    area: &mut [VoxelKind],
    x: i32,
    y: i32,
    z: i32,
    height: i32,
    padded: i32,
    kind: VoxelKind,
    replace: bool,
) {
    if !(-1..=padded - 2).contains(&x)
        || !(-1..=padded - 2).contains(&z)
        || !(0..height).contains(&y)
    {
        return;
    }
    let index = area_index(x, y, z, padded);
    if replace || area[index] == VoxelKind::Air {
        area[index] = kind;
    }
}

fn surface_kind(column: Column, slope: i32, sea: i32) -> VoxelKind {
    if column.height <= sea + 1 || column.biome == Biome::Desert {
        VoxelKind::Sand
    } else if slope >= 3 {
        VoxelKind::Stone
    } else if column.snow {
        VoxelKind::Snow
    } else {
        VoxelKind::Grass
    }
}

fn ground_kind(y: i32, height: i32, surface: VoxelKind) -> VoxelKind {
    if y == height - 1 {
        surface
    } else if y >= height - 4 {
        if surface == VoxelKind::Sand {
            VoxelKind::Sand
        } else if surface == VoxelKind::Stone {
            VoxelKind::Stone
        } else {
            VoxelKind::Dirt
        }
    } else {
        VoxelKind::Stone
    }
}

fn area_index(x: i32, y: i32, z: i32, padded: i32) -> usize {
    ((x + 1) + padded * ((z + 1) + padded * y)) as usize
}

fn sea_level(settings: &VoxelSettings) -> i32 {
    (settings.base_height.round() as i32).clamp(2, settings.max_height - 2)
}

fn simplex(seed: u32, frequency: f32) -> Simplex {
    let mut noise = Simplex::default();
    noise.set_seed(seed);
    noise.set_frequency(frequency);
    noise
}

fn smoothstep(min: f32, max: f32, value: f32) -> f32 {
    let t = ((value - min) / (max - min)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
#[path = "../voxel_generation_tests.rs"]
mod tests;
