//! Compact shared terrain records. Uniform sections never expand during decoding.
use crate::{
    VoxelState,
    coordinates::SECTION_VOLUME,
    sections::{Layer, Section},
};
use std::{io, sync::Arc};

pub struct TerrainRecord {
    pub states: Layer<VoxelState>,
    pub light: Layer<u8>,
    pub modified: bool,
}

fn invalid() -> io::Error {
    io::Error::other("Invalid or truncated terrain record.")
}

trait Cell: Copy + PartialEq {
    fn encode(self, bytes: &mut Vec<u8>);
    fn decode(bytes: &mut &[u8]) -> io::Result<Self>;
}
impl Cell for VoxelState {
    fn encode(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.id().to_le_bytes());
    }
    fn decode(bytes: &mut &[u8]) -> io::Result<Self> {
        Self::from_id(u16::from_le_bytes(take(bytes, 2)?.try_into().unwrap())).ok_or_else(invalid)
    }
}
impl Cell for u8 {
    fn encode(self, bytes: &mut Vec<u8>) {
        bytes.push(self);
    }
    fn decode(bytes: &mut &[u8]) -> io::Result<Self> {
        Ok(take(bytes, 1)?[0])
    }
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> io::Result<&'a [u8]> {
    let head = bytes.get(..count).ok_or_else(invalid)?;
    *bytes = &bytes[count..];
    Ok(head)
}
fn encode_section<T: Cell>(section: &Section<T>, bytes: &mut Vec<u8>) {
    match section {
        Section::Uniform(value) => {
            bytes.push(0);
            value.encode(bytes);
        }
        Section::Dense(cells) => {
            bytes.push(1);
            for &cell in cells.iter() {
                cell.encode(bytes);
            }
        }
    }
}
fn decode_section<T: Cell>(bytes: &mut &[u8]) -> io::Result<Section<T>> {
    match u8::decode(bytes)? {
        0 => Ok(Section::Uniform(T::decode(bytes)?)),
        1 => {
            let first = T::decode(bytes)?;
            let mut cells = Arc::new([first; SECTION_VOLUME]);
            for cell in &mut Arc::get_mut(&mut cells).unwrap()[1..] {
                *cell = T::decode(bytes)?;
            }
            Ok(Section::Dense(cells))
        }
        _ => Err(invalid()),
    }
}

impl TerrainRecord {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 * 1024);
        bytes.push(self.states.section_count() as u8);
        for section in 0..self.states.section_count() {
            encode_section(self.states.section(section), &mut bytes);
            encode_section(self.light.section(section), &mut bytes);
        }
        bytes.push(u8::from(self.modified));
        bytes
    }

    pub fn decode(mut bytes: &[u8], section_count: usize) -> io::Result<Self> {
        if u8::decode(&mut bytes)? as usize != section_count {
            return Err(invalid());
        }
        let mut states = Vec::with_capacity(section_count);
        let mut light = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            states.push(decode_section(&mut bytes)?);
            light.push(decode_section(&mut bytes)?);
        }
        let modified = match u8::decode(&mut bytes)? {
            0 => false,
            1 => true,
            _ => return Err(invalid()),
        };
        if !bytes.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            states: Layer::from_sections(states),
            light: Layer::from_sections(light),
            modified,
        })
    }
}

/// The database stores the stage as a typed field alongside the payload.
pub struct GenerationRecord {
    pub stage: crate::chunk_system::StagedChunkEnum,
    pub bytes: Vec<u8>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkingPayload {
    heights: Vec<Vec<i16>>,
    surface_estimates: Vec<(u64, i32)>,
    block_count: usize,
    block_entities: Vec<Vec<u8>>,
    structure_entities: Vec<Vec<u8>>,
    fluid_ticks: Vec<(String, [i32; 3], u8, i32)>,
    structure_starts: Vec<String>,
    structure_references: Vec<String>,
}
impl WorkingPayload {
    fn capture(chunk: &crate::ProtoChunk) -> Self {
        fn nbt(compound: &pumpkin_nbt::NbtCompound) -> Vec<u8> {
            pumpkin_nbt::Nbt::new(String::new(), compound.clone())
                .write()
                .to_vec()
        }
        let mut surface_estimates: Vec<_> = chunk
            .surface_estimates
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();
        surface_estimates.sort_unstable();
        let mut structure_starts: Vec<_> = chunk
            .structure_starts
            .keys()
            .map(|key| key.to_name().to_owned())
            .collect();
        structure_starts.sort_unstable();
        Self {
            heights: vec![
                chunk.flat_surface_height_map.to_vec(),
                chunk.flat_ocean_floor_height_map.to_vec(),
                chunk.flat_surface_wg_height_map.to_vec(),
                chunk.flat_ocean_floor_wg_height_map.to_vec(),
                chunk.flat_motion_blocking_height_map.to_vec(),
                chunk.flat_motion_blocking_no_leaves_height_map.to_vec(),
            ],
            surface_estimates,
            block_count: chunk.flat_block_map.len(),
            block_entities: chunk.pending_block_entities.iter().map(nbt).collect(),
            structure_entities: chunk.pending_structure_entities.iter().map(nbt).collect(),
            fluid_ticks: chunk
                .fluid_ticks
                .iter()
                .map(|tick| {
                    (
                        tick.value.name.to_owned(),
                        [tick.position.0.x, tick.position.0.y, tick.position.0.z],
                        tick.delay,
                        tick.priority as i32,
                    )
                })
                .collect(),
            structure_starts,
            structure_references: chunk
                .structure_references
                .iter()
                .map(|(key, _)| key.to_name().to_owned())
                .collect(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationHeader {
    position: [i32; 2],
    min_y: i32,
    height: i32,
    biomes: Vec<u8>,
    metadata: GenerationMetadata,
}

/// Completed columns retain no resumable generation metadata.
#[derive(serde::Serialize, serde::Deserialize)]
enum GenerationMetadata {
    Working(WorkingPayload),
    Complete(crate::chunk::ChunkHeightmaps),
}

pub(crate) fn encode_generated(chunk: &crate::chunk_system::Chunk) -> io::Result<GenerationRecord> {
    use crate::chunk_system::{Chunk, StagedChunkEnum as Stage};
    let (snapshot, biomes, metadata) = match chunk {
        Chunk::Proto(chunk) => (
            crate::ChunkSnapshot::from_proto(chunk, chunk.bottom_y() as i32, chunk.height() as i32),
            chunk.flat_biome_map.to_vec(),
            GenerationMetadata::Working(WorkingPayload::capture(chunk)),
        ),
        Chunk::Complete(chunk) => (
            chunk.snapshot(),
            chunk.section.biomes.to_vec(),
            GenerationMetadata::Complete(chunk.heightmap.lock().unwrap().clone()),
        ),
    };
    let header = GenerationHeader {
        position: [snapshot.chunk_x, snapshot.chunk_z],
        min_y: snapshot.min_y,
        height: snapshot.height,
        biomes,
        metadata,
    };
    let header = serde_json::to_vec(&header).map_err(io::Error::other)?;
    let terrain = TerrainRecord {
        states: snapshot.states,
        light: snapshot.light,
        modified: false,
    }
    .encode();
    let mut bytes = Vec::with_capacity(4 + header.len() + terrain.len());
    bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&terrain);
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(io::Error::other(
            "Generated chunk exceeds the record size limit.",
        ));
    }
    Ok(GenerationRecord {
        stage: Stage::from(chunk.get_stage_id()),
        bytes: compress_record(&bytes),
    })
}

pub(crate) fn decode_generated(
    record: GenerationRecord,
    position: crate::chunk_system::ChunkPos,
    generator: &crate::generation::generator::VanillaGenerator,
) -> io::Result<crate::chunk_system::Chunk> {
    use crate::{
        chunk::{ChunkData, ChunkHeightmaps, ChunkSections},
        chunk_system::{Chunk, StagedChunkEnum as Stage},
    };
    use std::sync::{Mutex, RwLock};
    let decoded = decompress_record(&record.bytes)?;
    let mut bytes = decoded.as_slice();
    let header_len = u32::from_le_bytes(take(&mut bytes, 4)?.try_into().unwrap()) as usize;
    let header: GenerationHeader =
        serde_json::from_slice(take(&mut bytes, header_len)?).map_err(io::Error::other)?;
    let dim = &generator.dimension;
    if header.position != [position.x, position.y]
        || header.min_y != dim.min_y
        || header.height != dim.height
        || record.stage < Stage::Empty
        || header.biomes.len() != dim.height as usize * 4
        || header
            .biomes
            .iter()
            .any(|&id| pumpkin_data::chunk::Biome::from_id(id).is_none())
    {
        return Err(invalid());
    }
    let terrain = TerrainRecord::decode(bytes, dim.height as usize / 16)?;
    if terrain.modified {
        return Err(invalid());
    }
    let metadata = match header.metadata {
        GenerationMetadata::Complete(heightmaps) => {
            if record.stage != Stage::Full
                || [
                    &heightmaps.world_surface,
                    &heightmaps.motion_blocking,
                    &heightmaps.motion_blocking_no_leaves,
                ]
                .into_iter()
                .flatten()
                .any(|heights| {
                    heights.len() != ChunkHeightmaps::LONGS
                        || heights.iter().any(|&word| {
                            word < 0
                                || (0..7)
                                    .any(|i| ((word as u64 >> (i * 9)) & 511) > dim.height as u64)
                        })
                })
            {
                return Err(invalid());
            }
            return Ok(Chunk::Complete(Arc::new(ChunkData {
                x: position.x,
                z: position.y,
                section: ChunkSections {
                    count: dim.height as usize / 16,
                    min_y: dim.min_y,
                    states: RwLock::new(terrain.states),
                    biomes: Arc::from(header.biomes),
                },
                heightmap: Mutex::new(heightmaps),
                light: Mutex::new(terrain.light),
            })));
        }
        GenerationMetadata::Working(metadata) => metadata,
    };
    if record.stage == Stage::Full
        || metadata.heights.len() != 6
        || metadata.heights.iter().any(|heights| {
            heights.len() != 256
                || heights.iter().any(|&y| {
                    y != i16::MIN
                        && (y < (dim.min_y - 1) as i16 || y >= (dim.min_y + dim.height) as i16)
                })
        })
        || (metadata.block_count != 0
            && (metadata.block_count > dim.height as usize * 256
                || metadata.block_count % SECTION_VOLUME != 0))
    {
        return Err(invalid());
    }
    for name in metadata
        .structure_starts
        .iter()
        .chain(metadata.structure_references.iter())
    {
        if pumpkin_data::structures::StructureKeys::from_name(name).is_none() {
            return Err(invalid());
        }
    }
    let read_nbt = |bytes: &Vec<u8>| -> io::Result<_> {
        let mut cursor = std::io::Cursor::new(bytes.as_slice());
        let nbt = pumpkin_nbt::Nbt::read(&mut pumpkin_nbt::deserializer::NbtReadHelperJava::new(
            &mut cursor,
        ))
        .map_err(io::Error::other)?;
        if cursor.position() != bytes.len() as u64 {
            return Err(invalid());
        }
        Ok(nbt.root_tag)
    };
    let block_entities = metadata
        .block_entities
        .iter()
        .map(read_nbt)
        .collect::<io::Result<_>>()?;
    let structure_entities = metadata
        .structure_entities
        .iter()
        .map(read_nbt)
        .collect::<io::Result<_>>()?;
    let fluid_ticks = metadata
        .fluid_ticks
        .iter()
        .map(|(name, p, delay, priority)| {
            let value = pumpkin_data::fluid::Fluid::from_registry_key(&name).ok_or_else(invalid)?;
            let priority = crate::tick::TickPriority::try_from(*priority).map_err(|_| invalid())?;
            if p[1] < dim.min_y || p[1] >= dim.min_y + dim.height {
                return Err(invalid());
            }
            Ok(crate::tick::ScheduledTick {
                value,
                position: pumpkin_util::math::position::BlockPos::new(p[0], p[1], p[2]),
                delay: *delay,
                priority,
            })
        })
        .collect::<io::Result<_>>()?;
    let mut chunk = crate::ProtoChunk::new(position.x, position.y, generator);
    if metadata.block_count != 0 {
        chunk.flat_block_map = (0..metadata.block_count)
            .map(|i| {
                let (x, y, z) = ((i % 4096) / 256, (i / 4096) * 16 + (i % 256) / 16, i % 16);
                pumpkin_data::BlockStateId::new(terrain.states.get(y * 256 + z * 16 + x).id())
                    .unwrap()
            })
            .collect();
    }
    chunk.flat_biome_map = header.biomes.into_boxed_slice();
    let heights: Vec<[i16; 256]> = metadata
        .heights
        .into_iter()
        .map(|h| h.try_into().unwrap())
        .collect();
    [
        chunk.flat_surface_height_map,
        chunk.flat_ocean_floor_height_map,
        chunk.flat_surface_wg_height_map,
        chunk.flat_ocean_floor_wg_height_map,
        chunk.flat_motion_blocking_height_map,
        chunk.flat_motion_blocking_no_leaves_height_map,
    ] = heights.try_into().unwrap();
    chunk.surface_estimates = metadata.surface_estimates.into_iter().collect();
    chunk.pending_block_entities = block_entities;
    chunk.pending_structure_entities = structure_entities;
    chunk.fluid_ticks = fluid_ticks;
    chunk.light = working_light(&terrain.light);
    // Structure collectors contain generation callbacks. Recreate their seeded recipes,
    // as the original resumable generator did, before restoring the exact saved stage.
    if record.stage >= Stage::StructureStart {
        chunk.stage = Stage::Biomes;
        chunk.set_structure_starts(generator);
    }
    if record.stage >= Stage::StructureReferences {
        chunk.set_structure_references(generator);
    }

    chunk.stage = record.stage;
    Ok(Chunk::Proto(Box::new(chunk)))
}

fn working_light(layer: &Layer<u8>) -> crate::chunk::ChunkLight {
    use crate::chunk::format::LightContainer;
    let channel = |sky| {
        (0..layer.section_count())
            .map(|s| {
                let extract = |value: u8| if sky { value >> 4 } else { value & 15 };
                match layer.section(s) {
                    Section::Uniform(value) => LightContainer::Empty(extract(*value)),
                    section => {
                        let mut light = LightContainer::new_empty(0);
                        for i in 0..SECTION_VOLUME {
                            light.set(i & 15, i >> 8, (i >> 4) & 15, extract(section.get(i)));
                        }
                        light
                    }
                }
            })
            .collect()
    };
    crate::chunk::ChunkLight {
        sky_light: channel(true),
        block_light: channel(false),
    }
}

pub const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
pub fn compress_record(bytes: &[u8]) -> Vec<u8> {
    let mut record = crc32fast::hash(bytes).to_le_bytes().to_vec();
    record.extend_from_slice(&lz4_flex::compress_prepend_size(bytes));
    record
}
pub fn decompress_record(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let checksum = bytes.get(..4).ok_or_else(invalid)?;
    let compressed = &bytes[4..];
    let size = compressed.get(..4).ok_or_else(invalid)?;
    if u32::from_le_bytes(size.try_into().unwrap()) as usize > MAX_RECORD_BYTES {
        return Err(invalid());
    }
    let record = lz4_flex::decompress_size_prepended(compressed).map_err(io::Error::other)?;
    if crc32fast::hash(&record) != u32::from_le_bytes(checksum.try_into().unwrap()) {
        return Err(invalid());
    }
    Ok(record)
}
