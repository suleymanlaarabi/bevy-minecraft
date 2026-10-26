use super::chunk_state::{Chunk, StagedChunkEnum};
use crate::ProtoChunk;
use crate::chunk::ChunkHeightmapType;
use crate::generation::biome_coords;
use crate::generation::generator;
use crate::generation::height_limit::HeightLimitView;
use crate::generation::proto_chunk::GenerationCache;
use crate::lighting::LightingEngineConfig;
use crate::world::{BlockAccessor, WorldPortalExt};
use pumpkin_data::biome::Biome;
use pumpkin_data::block_properties::is_air;
use pumpkin_data::fluid::{Fluid, FluidState};
use pumpkin_data::{Block, BlockState, BlockStateId};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::HeightMap;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use tracing::debug;

pub struct Cache {
    pub x: i32,
    pub z: i32,
    pub size: i32,
    pub chunks: Vec<Chunk>,
    surface_biomes: Option<Box<SurfaceBiomeNeighborhood>>,
}

struct SurfaceBiomePalette {
    chunk_x: i32,
    chunk_z: i32,
    bottom_quart_y: i32,
    height_quarts: usize,
    biomes: Box<[u8]>,
}

impl SurfaceBiomePalette {
    fn from_chunk(chunk: &Chunk) -> Option<Self> {
        match chunk {
            Chunk::Proto(chunk) => Some(Self {
                chunk_x: chunk.x,
                chunk_z: chunk.z,
                bottom_quart_y: biome_coords::from_block(chunk.bottom_y() as i32),
                height_quarts: chunk.height() as usize >> 2,
                biomes: chunk.flat_biome_map.clone(),
            }),
            Chunk::Complete(chunk) => {
                let bottom_y = chunk.section.min_y;
                let height_quarts = chunk.section.count * 4;
                let bottom_quart_y = biome_coords::from_block(bottom_y);
                let mut biomes = vec![0; 4 * height_quarts * 4];

                for local_x in 0..4 {
                    for local_y in 0..height_quarts {
                        for local_z in 0..4 {
                            let index = height_quarts * 4 * local_x + 4 * local_y + local_z;
                            biomes[index] = chunk.section.get_rough_biome_absolute_y(
                                local_x << 2,
                                biome_coords::to_block(bottom_quart_y + local_y as i32),
                                local_z << 2,
                            )?;
                        }
                    }
                }

                Some(Self {
                    chunk_x: chunk.x,
                    chunk_z: chunk.z,
                    bottom_quart_y,
                    height_quarts,
                    biomes: biomes.into_boxed_slice(),
                })
            }
        }
    }

    fn get_biome_id(&self, quart_x: i32, quart_y: i32, quart_z: i32) -> Option<u8> {
        if quart_x >> 2 != self.chunk_x || quart_z >> 2 != self.chunk_z {
            return None;
        }
        let local_y = quart_y - self.bottom_quart_y;
        if !(0..self.height_quarts as i32).contains(&local_y) {
            return None;
        }
        let local_x = (quart_x & 3) as usize;
        let local_z = (quart_z & 3) as usize;
        let index = self.height_quarts * 4 * local_x + 4 * local_y as usize + local_z;
        self.biomes.get(index).copied()
    }
}

pub(crate) struct SurfaceBiomeNeighborhood {
    center_x: i32,
    center_z: i32,
    palettes: [Option<SurfaceBiomePalette>; 9],
    // Uniform fuzzy-biome cubes need no seeded nearest-corner search.
    uniform: std::sync::OnceLock<Box<[u16]>>,
}

impl SurfaceBiomeNeighborhood {
    #[must_use]
    pub(crate) fn new(center_x: i32, center_z: i32) -> Self {
        Self {
            center_x,
            center_z,
            palettes: std::array::from_fn(|_| None),
            uniform: std::sync::OnceLock::new(),
        }
    }

    pub(crate) fn push_chunk(&mut self, chunk: &Chunk) -> bool {
        let Some(palette) = SurfaceBiomePalette::from_chunk(chunk) else {
            return false;
        };
        let dx = palette.chunk_x - self.center_x;
        let dz = palette.chunk_z - self.center_z;
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dz) {
            return false;
        }
        let slot = &mut self.palettes[((dx + 1) * 3 + dz + 1) as usize];
        if slot.is_some() {
            return false;
        }
        *slot = Some(palette);
        self.uniform.take();
        true
    }

    fn prepare_uniform_cubes(&self) -> Box<[u16]> {
        let center = self.palettes[4].as_ref().unwrap();
        let bottom = center.bottom_quart_y;
        let top = bottom + center.height_quarts as i32 - 1;
        let bottom_quart_y = bottom - 1;
        let cube_height = center.height_quarts + 1;
        let mut uniform = vec![u16::MAX; 25 * cube_height];
        for z in 0..5 {
            for x in 0..5 {
                for y in 0..cube_height {
                    let bx = self.center_x * 4 - 1 + x;
                    let by = bottom_quart_y + y as i32;
                    let bz = self.center_z * 4 - 1 + z;
                    let first = self.get_biome_id(bx, by.clamp(bottom, top), bz);
                    if let Some(first) = first
                        && (0..8).all(|corner| {
                            self.get_biome_id(
                                bx + ((corner >> 2) & 1),
                                (by + ((corner >> 1) & 1)).clamp(bottom, top),
                                bz + (corner & 1),
                            ) == Some(first)
                        })
                    {
                        uniform[y + (x + z * 5) as usize * cube_height] = u16::from(first);
                    }
                }
            }
        }
        uniform.into_boxed_slice()
    }

    pub(crate) fn uniform_biome(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        let center = self.palettes[4].as_ref()?;
        let bottom_quart_y = center.bottom_quart_y - 1;
        let cube_height = center.height_quarts + 1;
        let x = ((x - 2) >> 2) - (self.center_x * 4 - 1);
        let y = ((y - 2) >> 2) - bottom_quart_y;
        let z = ((z - 2) >> 2) - (self.center_z * 4 - 1);
        if !(0..5).contains(&x) || !(0..5).contains(&z) || !(0..cube_height as i32).contains(&y) {
            return None;
        }
        let uniform = self.uniform.get_or_init(|| self.prepare_uniform_cubes());
        let value = *uniform.get(y as usize + (x + z * 5) as usize * cube_height)?;
        u8::try_from(value).ok()
    }

    #[must_use]
    pub(crate) fn is_complete(&self) -> bool {
        self.palettes.iter().all(Option::is_some)
    }

    #[must_use]
    pub(crate) fn get_biome_id(&self, quart_x: i32, quart_y: i32, quart_z: i32) -> Option<u8> {
        let dx = (quart_x >> 2) - self.center_x;
        let dz = (quart_z >> 2) - self.center_z;
        if !(-1..=1).contains(&dx) || !(-1..=1).contains(&dz) {
            return None;
        }
        self.palettes[((dx + 1) * 3 + dz + 1) as usize]
            .as_ref()
            .and_then(|palette| palette.get_biome_id(quart_x, quart_y, quart_z))
    }
}

impl HeightLimitView for Cache {
    fn height(&self) -> u16 {
        let mid = ((self.size * self.size) >> 1) as usize;
        match &self.chunks[mid] {
            Chunk::Proto(chunk) => chunk.height(),
            Chunk::Complete(_) => panic!(),
        }
    }

    fn bottom_y(&self) -> i8 {
        let mid = ((self.size * self.size) >> 1) as usize;
        match &self.chunks[mid] {
            Chunk::Proto(chunk) => chunk.bottom_y(),
            Chunk::Complete(_) => panic!(),
        }
    }
}

impl BlockAccessor for Cache {
    fn get_block(&self, position: &BlockPos) -> &'static Block {
        GenerationCache::get_block_state(self, &position.0).to_block()
    }

    fn get_block_state(&self, position: &BlockPos) -> &'static BlockState {
        GenerationCache::get_block_state(self, &position.0).to_state()
    }

    fn get_block_state_id(&self, position: &BlockPos) -> BlockStateId {
        GenerationCache::get_block_state(self, &position.0)
    }

    fn get_block_and_state(&self, position: &BlockPos) -> (&'static Block, &'static BlockState) {
        let id = GenerationCache::get_block_state(self, &position.0);
        BlockState::from_id_with_block(id)
    }
}

impl GenerationCache for Cache {
    fn get_chunk_mut(&mut self, chunk_x: i32, chunk_z: i32) -> Option<&mut ProtoChunk> {
        let dx = chunk_x - self.x;
        let dz = chunk_z - self.z;

        if dx < 0 || dx >= self.size || dz < 0 || dz >= self.size {
            return None;
        }

        match &mut self.chunks[(dx * self.size + dz) as usize] {
            Chunk::Proto(chunk) => Some(chunk),
            Chunk::Complete(_) => None,
        }
    }

    fn get_chunk(&self, chunk_x: i32, chunk_z: i32) -> Option<&ProtoChunk> {
        let dx = chunk_x - self.x;
        let dz = chunk_z - self.z;

        if dx < 0 || dx >= self.size || dz < 0 || dz >= self.size {
            return None;
        }

        match &self.chunks[(dx * self.size + dz) as usize] {
            Chunk::Proto(chunk) => Some(chunk),
            Chunk::Complete(_) => None,
        }
    }

    fn try_get_proto_chunk(&self, chunk_x: i32, chunk_z: i32) -> Option<&ProtoChunk> {
        let dx = chunk_x - self.x;
        let dz = chunk_z - self.z;

        if dx < 0 || dx >= self.size || dz < 0 || dz >= self.size {
            return None;
        }

        match &self.chunks[(dx * self.size + dz) as usize] {
            Chunk::Proto(chunk) => Some(chunk),
            Chunk::Complete(_) => None,
        }
    }

    fn get_center_chunk(&self) -> &ProtoChunk {
        let mid = ((self.size * self.size) >> 1) as usize;
        self.chunks[mid].get_proto_chunk()
    }

    fn get_center_chunk_mut(&mut self) -> &mut ProtoChunk {
        let mid = ((self.size * self.size) >> 1) as usize;
        self.chunks[mid].get_proto_chunk_mut()
    }

    fn get_fluid_and_fluid_state(&self, pos: &Vector3<i32>) -> (Fluid, FluidState) {
        let id = GenerationCache::get_block_state(self, pos);

        let Some(fluid) = Fluid::from_state_id(id) else {
            let fluid = if id.is_waterlogged() {
                Fluid::FLOWING_WATER
            } else {
                Fluid::EMPTY
            };

            let state = fluid.states[0].clone();
            return (fluid, state);
        };

        //let state = fluid.get_state(id);
        let state = fluid.states[0].clone();

        (fluid.clone(), state)
    }

    fn get_block_state(&self, pos: &Vector3<i32>) -> BlockStateId {
        let dx = (pos.x >> 4) - self.x;
        let dz = (pos.z >> 4) - self.z;
        // debug_assert!(dx < self.size && dz < self.size);
        // debug_assert!(dx >= 0 && dz >= 0);
        if !(dx < self.size && dz < self.size && dx >= 0 && dz >= 0) {
            // breakpoint here
            debug!(
                "illegal get_block_state {pos:?} cache pos ({}, {}) size {}",
                self.x, self.z, self.size
            );
            return BlockStateId::AIR;
        }
        match &self.chunks[(dx * self.size + dz) as usize] {
            Chunk::Complete(data) => data
                .section
                .get_block_absolute_y((pos.x & 15) as usize, pos.y, (pos.z & 15) as usize)
                .unwrap_or(BlockStateId::AIR),

            Chunk::Proto(data) => data.get_block_state(pos),
        }
    }
    fn set_block_state(&mut self, pos: &Vector3<i32>, block_state: &BlockState) {
        let dx = (pos.x >> 4) - self.x;
        let dz = (pos.z >> 4) - self.z;
        // debug_assert!(dx < self.size && dz < self.size);
        // debug_assert!(dx >= 0 && dz >= 0);
        if !(dx < self.size && dz < self.size && dx >= 0 && dz >= 0) {
            // breakpoint here
            debug!(
                "illegal set_block_state {pos:?} cache pos ({}, {}) size {}",
                self.x, self.z, self.size
            );
            return;
        }
        match &mut self.chunks[(dx * self.size + dz) as usize] {
            Chunk::Complete(data) => {
                data.set_block_absolute_y(
                    (pos.x & 15) as usize,
                    pos.y,
                    (pos.z & 15) as usize,
                    block_state.id,
                );
            }
            Chunk::Proto(data) => {
                data.set_block_state(pos.x, pos.y, pos.z, block_state);
            }
        }
    }

    fn add_block_entity(&mut self, pos: &Vector3<i32>, nbt: NbtCompound) {
        let dx = (pos.x >> 4) - self.x;
        let dz = (pos.z >> 4) - self.z;
        if !(dx < self.size && dz < self.size && dx >= 0 && dz >= 0) {
            debug!(
                "illegal add_block_entity {pos:?} cache pos ({}, {}) size {}",
                self.x, self.z, self.size
            );
            return;
        }

        match &mut self.chunks[(dx * self.size + dz) as usize] {
            Chunk::Complete(_) => {
                debug!("add_block_entity on non-proto chunk at {pos:?}");
            }
            Chunk::Proto(data) => {
                data.add_block_entity(nbt);
            }
        }
    }

    fn get_top_y(&self, heightmap: &HeightMap, x: i32, z: i32) -> i32 {
        match heightmap {
            HeightMap::WorldSurfaceWg => self.top_block_wg_height_exclusive(x, z),
            HeightMap::WorldSurface => self.top_block_height_exclusive(x, z),
            HeightMap::OceanFloorWg => self.ocean_floor_wg_height_exclusive(x, z),
            HeightMap::OceanFloor => self.ocean_floor_height_exclusive(x, z),
            HeightMap::MotionBlocking => self.top_motion_blocking_block_height_exclusive(x, z),
            HeightMap::MotionBlockingNoLeaves => {
                self.top_motion_blocking_block_no_leaves_height_exclusive(x, z)
            }
        }
    }

    fn top_motion_blocking_block_height_exclusive(&self, x: i32, z: i32) -> i32 {
        let dx = (x >> 4) - self.x;
        let dy = (z >> 4) - self.z;
        debug_assert!(dx < self.size && dy < self.size);
        debug_assert!(dx >= 0 && dy >= 0);
        match &self.chunks[(dx * self.size + dy) as usize] {
            Chunk::Complete(data) => {
                let heightmap = data
                    .heightmap
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let min_y = data.section.min_y;

                heightmap.get(ChunkHeightmapType::MotionBlocking, x, z, min_y)
            }
            Chunk::Proto(data) => data.top_motion_blocking_block_height_exclusive(x, z),
        }
    }

    fn top_motion_blocking_block_no_leaves_height_exclusive(&self, x: i32, z: i32) -> i32 {
        let dx = (x >> 4) - self.x;
        let dy = (z >> 4) - self.z;
        debug_assert!(dx < self.size && dy < self.size);
        debug_assert!(dx >= 0 && dy >= 0);
        match &self.chunks[(dx * self.size + dy) as usize] {
            Chunk::Complete(data) => {
                let heightmap = data
                    .heightmap
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let min_y = data.section.min_y;
                heightmap.get(ChunkHeightmapType::MotionBlockingNoLeaves, x, z, min_y)
            }
            Chunk::Proto(data) => data.top_motion_blocking_block_no_leaves_height_exclusive(x, z),
        }
    }

    fn top_block_height_exclusive(&self, x: i32, z: i32) -> i32 {
        let dx = (x >> 4) - self.x;
        let dy = (z >> 4) - self.z;
        debug_assert!(dx < self.size && dy < self.size);
        debug_assert!(dx >= 0 && dy >= 0);
        match &self.chunks[(dx * self.size + dy) as usize] {
            Chunk::Complete(data) => {
                let heightmap = data
                    .heightmap
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let min_y = data.section.min_y;
                heightmap.get(ChunkHeightmapType::WorldSurface, x, z, min_y) // can we return this?
            }
            Chunk::Proto(data) => data.top_block_height_exclusive(x, z),
        }
    }

    fn top_block_wg_height_exclusive(&self, x: i32, z: i32) -> i32 {
        let dx = (x >> 4) - self.x;
        let dy = (z >> 4) - self.z;
        debug_assert!(dx < self.size && dy < self.size);
        debug_assert!(dx >= 0 && dy >= 0);
        match &self.chunks[(dx * self.size + dy) as usize] {
            Chunk::Complete(data) => {
                let heightmap = data
                    .heightmap
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let min_y = data.section.min_y;
                heightmap.get(ChunkHeightmapType::WorldSurface, x, z, min_y)
            }
            Chunk::Proto(data) => data.top_block_wg_height_exclusive(x, z),
        }
    }

    fn ocean_floor_height_exclusive(&self, x: i32, z: i32) -> i32 {
        let dx = (x >> 4) - self.x;
        let dy = (z >> 4) - self.z;
        if dx < 0 || dy < 0 || dx >= self.size || dy >= self.size {
            return 0;
        }
        match &self.chunks[(dx * self.size + dy) as usize] {
            Chunk::Complete(_data) => {
                0 // todo missing
            }
            Chunk::Proto(data) => data.ocean_floor_height_exclusive(x, z),
        }
    }

    fn ocean_floor_wg_height_exclusive(&self, x: i32, z: i32) -> i32 {
        let dx = (x >> 4) - self.x;
        let dy = (z >> 4) - self.z;
        if dx < 0 || dy < 0 || dx >= self.size || dy >= self.size {
            return 0;
        }
        match &self.chunks[(dx * self.size + dy) as usize] {
            Chunk::Complete(_data) => 0,
            Chunk::Proto(data) => data.ocean_floor_wg_height_exclusive(x, z),
        }
    }

    fn get_biome_for_terrain_gen(&self, x: i32, y: i32, z: i32) -> &'static Biome {
        let biome_pos = self.get_center_chunk().get_terrain_gen_biome_pos(x, y, z);
        let dx = (biome_pos.x >> 2) - self.x;
        let dz = (biome_pos.z >> 2) - self.z;
        let (dx, dz) = if dx < 0 || dz < 0 || dx >= self.size || dz >= self.size {
            // Position is outside the cache — fall back to the centre chunk's biome
            let mid = self.size / 2;
            (mid, mid)
        } else {
            (dx, dz)
        };
        match &self.chunks[(dx * self.size + dz) as usize] {
            Chunk::Complete(data) => {
                // Could this happen?
                Biome::from_id(
                    data.section
                        .get_rough_biome_absolute_y(
                            (biome_coords::to_block(biome_pos.x) & 15) as usize,
                            biome_coords::to_block(biome_pos.y),
                            (biome_coords::to_block(biome_pos.z) & 15) as usize,
                        )
                        .unwrap_or(0),
                )
                .unwrap_or(&Biome::PLAINS)
            }
            Chunk::Proto(data) => data.get_biome(biome_pos.x, biome_pos.y, biome_pos.z),
        }
    }

    fn is_air(&self, local_pos: &Vector3<i32>) -> bool {
        is_air(GenerationCache::get_block_state(self, local_pos))
    }

    fn get_sea_level(&self) -> i32 {
        self.get_center_chunk().get_sea_level()
    }
}

impl Cache {
    pub fn advance_all(
        &mut self,
        stage: StagedChunkEnum,
        generator: &generator::WorldGenerator,
        block_registry: &dyn WorldPortalExt,
        lighting_config: &LightingEngineConfig,
    ) {
        for index in 0..self.chunks.len() {
            self.advance_index(index, stage, generator, block_registry, lighting_config);
        }
    }

    pub fn advance_index(
        &mut self,
        index: usize,
        stage: StagedChunkEnum,
        generator: &generator::WorldGenerator,
        _block_registry: &dyn WorldPortalExt,
        _lighting_config: &LightingEngineConfig,
    ) {
        match &self.chunks[index] {
            Chunk::Complete(_) => return,
            Chunk::Proto(chunk) if chunk.stage >= stage => return,
            Chunk::Proto(_) => {}
        }
        match stage {
            StagedChunkEnum::Empty => panic!("empty stage"),
            StagedChunkEnum::StructureStart => match generator {
                generator::WorldGenerator::Noise(noise_gen) => {
                    self.chunks[index]
                        .get_proto_chunk_mut()
                        .set_structure_starts(noise_gen);
                }
                generator::WorldGenerator::Flat(_) => {}
                generator::WorldGenerator::Custom(custom_gen) => {
                    custom_gen.set_structure_starts(self.chunks[index].get_proto_chunk_mut());
                }
            },
            StagedChunkEnum::StructureReferences => match generator {
                generator::WorldGenerator::Noise(noise_gen) => {
                    self.chunks[index]
                        .get_proto_chunk_mut()
                        .set_structure_references(noise_gen);
                }
                generator::WorldGenerator::Flat(_) => {}
                generator::WorldGenerator::Custom(custom_gen) => {
                    custom_gen.set_structure_references(self.chunks[index].get_proto_chunk_mut());
                }
            },
            StagedChunkEnum::Biomes => match generator {
                generator::WorldGenerator::Noise(noise_gen) => {
                    self.chunks[index]
                        .get_proto_chunk_mut()
                        .step_to_biomes(noise_gen);
                }
                generator::WorldGenerator::Flat(flat_gen) => {
                    flat_gen.step_to_biomes(self.chunks[index].get_proto_chunk_mut());
                }
                generator::WorldGenerator::Custom(custom_gen) => {
                    custom_gen.step_to_biomes(self.chunks[index].get_proto_chunk_mut());
                }
            },
            _ => return,
        }
        self.chunks[index].get_proto_chunk_mut().stage = stage;
    }

    #[must_use]
    pub fn new(x: i32, z: i32, size: i32) -> Self {
        Self {
            x,
            z,
            size,
            chunks: Vec::with_capacity((size * size) as usize),
            surface_biomes: None,
        }
    }

    pub(crate) fn set_surface_biomes(&mut self, biomes: SurfaceBiomeNeighborhood) {
        debug_assert!(biomes.is_complete());
        self.surface_biomes = Some(Box::new(biomes));
    }

    fn prepare_surface_biomes(&mut self) {
        if self.surface_biomes.is_some() || self.size < 3 {
            return;
        }
        let center_x = self.x + self.size / 2;
        let center_z = self.z + self.size / 2;
        let mut neighborhood = SurfaceBiomeNeighborhood::new(center_x, center_z);
        for chunk_x in center_x - 1..=center_x + 1 {
            for chunk_z in center_z - 1..=center_z + 1 {
                let dx = chunk_x - self.x;
                let dz = chunk_z - self.z;
                let index = (dx * self.size + dz) as usize;
                if !neighborhood.push_chunk(&self.chunks[index]) {
                    return;
                }
            }
        }
        self.surface_biomes = Some(Box::new(neighborhood));
    }
    #[allow(clippy::too_many_lines)]
    pub fn advance(
        &mut self,
        stage: StagedChunkEnum,
        generator: &generator::WorldGenerator,
        block_registry: &dyn WorldPortalExt,
        lighting_config: &LightingEngineConfig,
    ) {
        if stage == StagedChunkEnum::Surface {
            self.prepare_surface_biomes();
        }
        let mid = ((self.size * self.size) >> 1) as usize;
        match &self.chunks[mid] {
            Chunk::Complete(_) => return,
            Chunk::Proto(chunk) if chunk.stage >= stage => return,
            Chunk::Proto(_) => {}
        }
        match stage {
            StagedChunkEnum::Empty => panic!("empty stage"),
            StagedChunkEnum::Biomes
            | StagedChunkEnum::StructureStart
            | StagedChunkEnum::StructureReferences => {
                self.advance_index(mid, stage, generator, block_registry, lighting_config);
            }
            StagedChunkEnum::Noise => match generator {
                generator::WorldGenerator::Noise(noise_gen) => {
                    self.chunks[mid]
                        .get_proto_chunk_mut()
                        .step_to_noise(noise_gen);
                }
                generator::WorldGenerator::Flat(flat_gen) => {
                    flat_gen.step_to_noise(self.chunks[mid].get_proto_chunk_mut());
                }
                generator::WorldGenerator::Custom(custom_gen) => {
                    custom_gen.step_to_noise(self.chunks[mid].get_proto_chunk_mut());
                }
            },
            StagedChunkEnum::Surface => match generator {
                generator::WorldGenerator::Noise(noise_gen) => {
                    let surface_biomes = self
                        .surface_biomes
                        .take()
                        .expect("surface stage requires a complete biome neighborhood");
                    self.chunks[mid]
                        .get_proto_chunk_mut()
                        .step_to_surface(noise_gen, &surface_biomes);
                }
                generator::WorldGenerator::Flat(flat_gen) => {
                    flat_gen.step_to_surface(self.chunks[mid].get_proto_chunk_mut());
                }
                generator::WorldGenerator::Custom(custom_gen) => {
                    custom_gen.step_to_surface(self.chunks[mid].get_proto_chunk_mut());
                }
            },
            StagedChunkEnum::Carvers => match generator {
                generator::WorldGenerator::Noise(noise_gen) => {
                    self.chunks[mid]
                        .get_proto_chunk_mut()
                        .step_to_carvers(noise_gen);
                }
                generator::WorldGenerator::Flat(flat_gen) => {
                    flat_gen.step_to_carvers(self.chunks[mid].get_proto_chunk_mut());
                }
                generator::WorldGenerator::Custom(custom_gen) => {
                    custom_gen.step_to_carvers(self.chunks[mid].get_proto_chunk_mut());
                }
            },
            StagedChunkEnum::Features => match generator {
                generator::WorldGenerator::Noise(noise_gen) => {
                    ProtoChunk::generate_features_and_structure(
                        self,
                        block_registry,
                        &noise_gen.random_config,
                    );
                }
                generator::WorldGenerator::Flat(_) => {
                    self.chunks[mid].get_proto_chunk_mut().stage = StagedChunkEnum::Features;
                }
                generator::WorldGenerator::Custom(custom_gen) => {
                    custom_gen.step_to_features(self, block_registry);
                }
            },
            StagedChunkEnum::Lighting => {
                crate::lighting::engine::initialize_generation_light(self, lighting_config);
                // Only set stage to Lighting if it wasn't already at Lighting or higher
                // (initialize_light may short-circuit for already-lit chunks)
                let chunk = self.chunks[mid].get_proto_chunk_mut();
                if chunk.stage < StagedChunkEnum::Lighting {
                    chunk.stage = StagedChunkEnum::Lighting;
                }
            }
            StagedChunkEnum::Spawn => {
                ProtoChunk::spawn_mobs(self, block_registry);
            }
            StagedChunkEnum::Full => {
                let chunk = self.chunks[mid].get_proto_chunk_mut();
                debug_assert_eq!(chunk.stage, StagedChunkEnum::Spawn);
                chunk.stage = StagedChunkEnum::Full;
                let chunk = self
                    .chunks
                    .remove(mid)
                    .finalize(generator.dimension(), lighting_config);
                self.chunks.insert(mid, chunk);
            }
            StagedChunkEnum::None => {}
        }
    }
}
