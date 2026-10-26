use crate::ProtoChunk;
use crate::generation::generator::VanillaGenerator;
use crate::lighting::LightingEngineConfig;
use crate::world::WorldPortalExt;

use super::{Cache, Chunk, StagedChunkEnum};

pub fn generate_single_chunk(
    generator: &VanillaGenerator,
    block_registry: &dyn WorldPortalExt,
    chunk_x: i32,
    chunk_z: i32,
    target_stage: StagedChunkEnum,
) -> Chunk {
    generate_single_chunk_with_radius(
        generator,
        block_registry,
        chunk_x,
        chunk_z,
        target_stage,
        target_stage.get_direct_radius(),
    )
}

pub fn generate_single_chunk_with_radius(
    generator: &VanillaGenerator,
    block_registry: &dyn WorldPortalExt,
    chunk_x: i32,
    chunk_z: i32,
    target_stage: StagedChunkEnum,
    radius: i32,
) -> Chunk {
    let mut cache = Cache::new(chunk_x - radius, chunk_z - radius, radius * 2 + 1);

    for dx in -radius..=radius {
        for dz in -radius..=radius {
            let new_x = chunk_x + dx;
            let new_z = chunk_z + dz;

            let proto_chunk = Box::new(ProtoChunk::new(new_x, new_z, generator));

            cache.chunks.push(Chunk::Proto(proto_chunk));
        }
    }

    let stages = [
        StagedChunkEnum::Biomes,
        StagedChunkEnum::StructureStart,
        StagedChunkEnum::StructureReferences,
        StagedChunkEnum::Noise,
        StagedChunkEnum::Surface,
        StagedChunkEnum::Carvers,
        StagedChunkEnum::Features,
        StagedChunkEnum::Lighting,
        StagedChunkEnum::Full,
    ];

    for &stage in &stages {
        if stage as u8 > target_stage as u8 {
            break;
        }

        if matches!(
            stage,
            StagedChunkEnum::Biomes
                | StagedChunkEnum::StructureStart
                | StagedChunkEnum::StructureReferences
        ) {
            cache.advance_all(stage, generator);
        } else {
            cache.advance(
                stage,
                generator,
                block_registry,
                &LightingEngineConfig::Default,
            );
        }
    }

    let mid = ((cache.size * cache.size) >> 1) as usize;
    cache.chunks.swap_remove(mid)
}
