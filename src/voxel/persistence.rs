//! Game presentation state is reconstructed around the shared terrain codec.
use super::ChunkVoxels;
use hollow_worldgen::{codec::TerrainRecord, coordinates::SECTION_COUNT};
use std::io;

impl ChunkVoxels {
    pub(crate) fn encode_saved(&self) -> Vec<u8> {
        TerrainRecord {
            states: self.states.clone(),
            light: self.light.clone(),
            edits: self.native_edits.clone(),
        }
        .encode()
    }
    pub(crate) fn decode_saved(bytes: &[u8]) -> io::Result<Self> {
        let record = TerrainRecord::decode(bytes, SECTION_COUNT)?;
        let mut chunk = Self::from_layers(record.states, record.light);
        chunk.modified = !record.edits.is_empty();
        chunk.native_edits = record.edits;
        chunk.save_revision = 1;
        chunk.saved_revision = 1;
        Ok(chunk)
    }
}

