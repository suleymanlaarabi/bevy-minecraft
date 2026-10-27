//! Compact terrain records over shared sections, independent of game content.
use crate::{
    VoxelCell,
    coordinates::SECTION_VOLUME,
    sections::{Layer, Section},
};
use std::{io, sync::Arc};

pub struct TerrainRecord<T: VoxelCell> {
    pub states: Layer<T>,
    pub light: Layer<u8>,
    pub modified: bool,
}

fn invalid() -> io::Error {
    io::Error::other("Invalid or truncated terrain record.")
}

fn take<'a>(bytes: &mut &'a [u8], count: usize) -> io::Result<&'a [u8]> {
    let head = bytes.get(..count).ok_or_else(invalid)?;
    *bytes = &bytes[count..];
    Ok(head)
}
fn encode_section<T: Copy>(
    section: &Section<T>,
    bytes: &mut Vec<u8>,
    encode: impl Fn(T, &mut Vec<u8>),
) {
    match section {
        Section::Uniform(value) => {
            bytes.push(0);
            encode(*value, bytes);
        }
        Section::Dense(cells) => {
            bytes.push(1);
            for &cell in cells.iter() {
                encode(cell, bytes);
            }
        }
    }
}
fn decode_section<T: Copy>(
    bytes: &mut &[u8],
    decode: impl Fn(&mut &[u8]) -> io::Result<T>,
) -> io::Result<Section<T>> {
    match take(bytes, 1)?[0] {
        0 => Ok(Section::Uniform(decode(bytes)?)),
        1 => {
            let first = decode(bytes)?;
            let mut cells = Arc::new([first; SECTION_VOLUME]);
            for cell in &mut Arc::get_mut(&mut cells).unwrap()[1..] {
                *cell = decode(bytes)?;
            }
            Ok(Section::Dense(cells))
        }
        _ => Err(invalid()),
    }
}

impl<T: VoxelCell> TerrainRecord<T> {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 * 1024);
        bytes.push(
            u8::try_from(self.states.section_count())
                .expect("Section count exceeds terrain storage"),
        );
        for section in 0..self.states.section_count() {
            encode_section(self.states.section(section), &mut bytes, |state, bytes| {
                bytes.extend_from_slice(&state.id().to_le_bytes())
            });
            encode_section(self.light.section(section), &mut bytes, |light, bytes| {
                bytes.push(light)
            });
        }
        bytes.push(u8::from(self.modified));
        bytes
    }
    pub fn decode(mut bytes: &[u8], section_count: usize) -> io::Result<Self> {
        if take(&mut bytes, 1)?[0] as usize != section_count {
            return Err(invalid());
        }
        let mut states = Vec::with_capacity(section_count);
        let mut light = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            states.push(decode_section(&mut bytes, |bytes| {
                T::from_id(u32::from_le_bytes(take(bytes, 4)?.try_into().unwrap()))
                    .ok_or_else(invalid)
            })?);
            light.push(decode_section(&mut bytes, |bytes| Ok(take(bytes, 1)?[0]))?);
        }
        let modified = match take(&mut bytes, 1)?[0] {
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
