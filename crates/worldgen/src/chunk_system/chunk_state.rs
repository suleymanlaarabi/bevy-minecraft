use crate::chunk::{ChunkData, ChunkHeightmapType, ChunkHeightmaps, ChunkSections};
use crate::lighting::LightingEngineConfig;
use pumpkin_data::dimension::Dimension;
use std::sync::Arc;

use crate::ProtoChunk;

use std::sync::Mutex;

#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum StagedChunkEnum {
    None,
    /// Initial empty chunk, ready for biome population
    Empty = 1, // EMPTY STRUCTURE_STARTS STRUCTURE_REFERENCES
    /// Chunk with biomes populated, ready for noise generation
    Biomes,
    StructureStart,
    StructureReferences,
    /// Chunk with terrain noise generated, ready for surface building
    Noise,
    /// Chunk with surface built, ready for carvers
    Surface,
    /// Chunk with carvers applied, ready for features and structures
    Carvers,
    /// Chunk with features and structures, ready for lighting
    Features, // FEATURES
    /// Chunk with lighting calculated, ready for spawning
    Lighting, // INITIALIZE LIGHT
    /// Chunk with mobs spawned, ready for finalization
    Spawn, // SPAWN
    /// Fully generated chunk
    Full,
}

#[expect(clippy::fallible_impl_from)]
impl From<u8> for StagedChunkEnum {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Empty,
            2 => Self::Biomes,
            3 => Self::StructureStart,
            4 => Self::StructureReferences,
            5 => Self::Noise,
            6 => Self::Surface,
            7 => Self::Carvers,
            8 => Self::Features,
            9 => Self::Lighting,
            10 => Self::Spawn,
            11 => Self::Full,
            _ => panic!(),
        }
    }
}

impl StagedChunkEnum {
    /// Total number of state values (0 = None … 11 = Full).
    pub const COUNT: usize = Self::Full as usize + 1;
    #[must_use]
    pub const fn get_direct_radius(self) -> i32 {
        // self exclude
        match self {
            Self::Surface | Self::Features | Self::Lighting | Self::Spawn | Self::Full => 1,
            _ => 0,
        }
    }
    #[must_use]
    pub const fn get_read_radius(self) -> i32 {
        match self {
            Self::Surface => 1,
            _ => self.get_write_radius(),
        }
    }
    #[must_use]
    pub const fn get_write_radius(self) -> i32 {
        // self exclude
        match self {
            Self::Features | Self::Lighting | Self::Spawn => 1,
            _ => 0,
        }
    }
    #[must_use]
    pub const fn get_direct_dependencies(self) -> &'static [Self] {
        match self {
            // In vanilla StructureStart is first, but since it needs the biome in Vanilla it gets computed in StructureStart and
            // the Biome Step, this should be more efficient
            Self::Biomes => &[Self::Empty],
            Self::StructureStart => &[Self::Biomes],
            Self::StructureReferences => &[
                Self::StructureStart,
                Self::StructureStart,
                Self::StructureStart,
                Self::StructureStart,
                Self::StructureStart,
                Self::StructureStart,
                Self::StructureStart,
                Self::StructureStart,
                Self::StructureStart,
            ],
            Self::Noise => &[Self::StructureReferences],
            Self::Surface => &[Self::Noise, Self::Biomes],
            Self::Carvers => &[Self::Surface],
            Self::Features => &[Self::Carvers, Self::Carvers],
            Self::Lighting => &[Self::Features, Self::Features],
            Self::Spawn => &[Self::Lighting, Self::Lighting],
            Self::Full => &[Self::Spawn, Self::Spawn],
            _ => panic!(),
        }
    }
}

pub enum Chunk {
    Complete(Arc<ChunkData>),
    Proto(Box<ProtoChunk>),
}

impl Chunk {
    #[must_use]
    pub fn get_stage_id(&self) -> u8 {
        match self {
            Self::Proto(data) => data.stage_id(),
            Self::Complete(_) => StagedChunkEnum::Full as u8,
        }
    }
    pub fn get_proto_chunk_mut(&mut self) -> &mut ProtoChunk {
        match self {
            Self::Complete(_) => panic!("chunk isn't a ProtoChunk"),
            Self::Proto(chunk) => chunk,
        }
    }
    #[must_use]
    pub fn get_proto_chunk(&self) -> &ProtoChunk {
        match self {
            Self::Complete(_) => panic!("chunk isn't a ProtoChunk"),
            Self::Proto(chunk) => chunk,
        }
    }

    fn build_heightmaps(proto_chunk: &ProtoChunk, min_y: i32) -> ChunkHeightmaps {
        let mut heightmaps = ChunkHeightmaps::default();
        for x in 0..16 {
            for z in 0..16 {
                let source_index = x * 16 + z;
                for (heightmap_type, height) in [
                    (
                        ChunkHeightmapType::WorldSurface,
                        proto_chunk.flat_surface_height_map[source_index],
                    ),
                    (
                        ChunkHeightmapType::MotionBlocking,
                        proto_chunk.flat_motion_blocking_height_map[source_index],
                    ),
                    (
                        ChunkHeightmapType::MotionBlockingNoLeaves,
                        proto_chunk.flat_motion_blocking_no_leaves_height_map[source_index],
                    ),
                ] {
                    heightmaps.set(heightmap_type, x as i32, z as i32, i32::from(height), min_y);
                }
            }
        }
        heightmaps
    }

    pub fn finalize(self, dimension: &Dimension, _lighting_config: &LightingEngineConfig) -> Self {
        let Self::Proto(proto) = self else {
            panic!("Only working chunks can be finalized")
        };
        let snapshot = crate::ChunkSnapshot::from_proto(&proto, dimension.min_y, dimension.height);
        let heightmaps = Self::build_heightmaps(&proto, dimension.min_y);
        let metadata = Arc::new(crate::codec::GenerationPayload::capture(&proto));
        Self::Complete(Arc::new(ChunkData {
            x: proto.x,
            z: proto.z,
            section: ChunkSections {
                count: dimension.height as usize / 16,
                min_y: dimension.min_y,
                states: std::sync::RwLock::new(snapshot.states),
                biomes: proto.flat_biome_map.into(),
            },
            heightmap: Mutex::new(heightmaps),
            light: Mutex::new(snapshot.light),
            metadata,
        }))
    }
}
