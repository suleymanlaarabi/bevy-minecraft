//! Compact section snapshots. Encoding and decoding happen on storage workers.
use super::{
    ChunkVoxels, VoxelKind, VoxelState,
    coordinates::{CHUNK_VOLUME, SECTION_COUNT, SECTION_VOLUME},
    storage::Section,
};
use std::{collections::BTreeSet, io, sync::Arc};

impl ChunkVoxels {
    pub(crate) fn encode_saved(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 * 1024);
        bytes.extend_from_slice(b"HCHK\x01");
        let states = self
            .native_states
            .as_ref()
            .expect("persistent chunks have native states");
        for s in 0..SECTION_COUNT {
            match states.section(s) {
                Section::Uniform(value) => {
                    bytes.push(0);
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                Section::Dense(cells) => {
                    bytes.push(1);
                    for state in cells.iter() {
                        bytes.extend_from_slice(&state.to_le_bytes());
                    }
                }
            }
            match self.light.section(s) {
                Section::Uniform(value) => {
                    bytes.push(0);
                    bytes.push(*value);
                }
                Section::Dense(cells) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&cells[..]);
                }
            }
        }
        bytes.extend_from_slice(&(self.native_edits.len() as u32).to_le_bytes());
        for &index in self.native_edits.iter() {
            bytes.extend_from_slice(&(index as u32).to_le_bytes());
        }
        bytes
    }

    pub(crate) fn decode_saved(bytes: &[u8]) -> io::Result<Self> {
        let invalid = || io::Error::other("Invalid or truncated chunk snapshot.");
        let mut bytes = bytes;
        fn take<'a>(bytes: &mut &'a [u8], len: usize) -> io::Result<&'a [u8]> {
            if bytes.len() < len {
                return Err(io::Error::other("Truncated chunk snapshot."));
            }
            let (head, rest) = bytes.split_at(len);
            *bytes = rest;
            Ok(head)
        }
        if take(&mut bytes, 5)? != b"HCHK\x01" {
            return Err(invalid());
        }
        let mut states = Vec::with_capacity(CHUNK_VOLUME);
        let mut light = Vec::with_capacity(CHUNK_VOLUME);
        for _ in 0..SECTION_COUNT {
            match take(&mut bytes, 1)?[0] {
                0 => {
                    let state = u16::from_le_bytes(take(&mut bytes, 2)?.try_into().unwrap());
                    if VoxelState::from_id(state).is_none() {
                        return Err(invalid());
                    }
                    states.extend(std::iter::repeat_n(state, SECTION_VOLUME));
                }
                1 => {
                    for state in take(&mut bytes, SECTION_VOLUME * 2)?.chunks_exact(2) {
                        let state = u16::from_le_bytes(state.try_into().unwrap());
                        if VoxelState::from_id(state).is_none() {
                            return Err(invalid());
                        }
                        states.push(state);
                    }
                }
                _ => return Err(invalid()),
            }
            match take(&mut bytes, 1)?[0] {
                0 => {
                    let value = take(&mut bytes, 1)?[0];
                    light.extend(std::iter::repeat_n(value, SECTION_VOLUME));
                }
                1 => light.extend_from_slice(take(&mut bytes, SECTION_VOLUME)?),
                _ => return Err(invalid()),
            }
        }
        let count = u32::from_le_bytes(take(&mut bytes, 4)?.try_into().unwrap()) as usize;
        if count > CHUNK_VOLUME {
            return Err(invalid());
        }
        let mut edits = BTreeSet::new();
        for index in take(&mut bytes, count * 4)?.chunks_exact(4) {
            let index = u32::from_le_bytes(index.try_into().unwrap()) as usize;
            if index >= CHUNK_VOLUME || !edits.insert(index) {
                return Err(invalid());
            }
        }
        if !bytes.is_empty() {
            return Err(invalid());
        }
        let kinds: Vec<_> = states
            .iter()
            .map(|&s| VoxelState::from_id(s).unwrap().kind())
            .collect::<Vec<VoxelKind>>();
        let mut chunk = Self::generated(&kinds, &states, &light);
        chunk.modified = !edits.is_empty();
        chunk.native_edits = Arc::new(edits);
        chunk.save_revision = 1;
        chunk.saved_revision = 1;
        Ok(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::super::storage::Layer;
    use super::*;
    use bevy::prelude::*;

    #[test]
    fn snapshots_preserve_native_states_edits_and_light() {
        let mut chunk = ChunkVoxels::filled(VoxelKind::Air);
        let state = VoxelKind::from_key("oak_stairs")
            .unwrap()
            .default_state()
            .with(&[("facing", "west"), ("half", "top"), ("waterlogged", "true")]);
        let cell = IVec3::new(15, 383, 0);
        chunk.set_state(cell, state);
        chunk.light = Layer::filled(0x7b);
        let restored = ChunkVoxels::decode_saved(&chunk.encode_saved()).unwrap();
        assert_eq!(restored.state(cell), Some(state));
        assert_eq!(restored.light.get(0), 0x7b);
        assert_eq!(*restored.native_edits, *chunk.native_edits);
        assert_eq!(restored.save_revision, restored.saved_revision);
        assert!(restored.collider_dirty);
        assert_eq!(
            restored.dirty_sections,
            super::super::coordinates::ALL_SECTIONS
        );
    }

    #[test]
    fn snapshots_reject_truncation_and_unknown_versions() {
        let mut chunk = ChunkVoxels::filled(VoxelKind::Air);
        chunk.set(IVec3::ZERO, VoxelKind::Stone);
        let mut bytes = chunk.encode_saved();
        for len in [0, 4, 5, bytes.len() / 2, bytes.len() - 1] {
            assert!(ChunkVoxels::decode_saved(&bytes[..len]).is_err());
        }
        bytes[4] = 2;
        assert!(ChunkVoxels::decode_saved(&bytes).is_err());
    }
}
