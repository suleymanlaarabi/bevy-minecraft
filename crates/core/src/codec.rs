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
    /// Appends the record of borrowed layers to `bytes`, reserving its exact size once
    /// and copying dense cells in bulk.
    pub fn encode(states: &Layer<T>, light: &Layer<u8>, modified: bool, bytes: &mut Vec<u8>) {
        let count = states.section_count();
        let section_bytes =
            |dense: bool, cell: usize| 1 + if dense { SECTION_VOLUME * cell } else { cell };
        bytes.reserve_exact(
            2 + (0..count)
                .map(|section| {
                    section_bytes(states.section(section).uniform().is_none(), 4)
                        + section_bytes(light.section(section).uniform().is_none(), 1)
                })
                .sum::<usize>(),
        );
        bytes.push(u8::try_from(count).expect("Section count exceeds terrain storage"));
        for section in 0..count {
            match states.section(section) {
                Section::Uniform(state) => {
                    bytes.push(0);
                    bytes.extend_from_slice(&state.id().to_le_bytes());
                }
                Section::Dense(cells) => {
                    bytes.push(1);
                    let start = bytes.len();
                    bytes.resize(start + 4 * SECTION_VOLUME, 0);
                    for (out, cell) in bytes[start..].chunks_exact_mut(4).zip(cells.iter()) {
                        out.copy_from_slice(&cell.id().to_le_bytes());
                    }
                }
            }
            match light.section(section) {
                Section::Uniform(level) => bytes.extend_from_slice(&[0, *level]),
                Section::Dense(cells) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&cells[..]);
                }
            }
        }
        bytes.push(u8::from(modified));
    }
    pub fn decode(
        mut bytes: &[u8],
        section_count: usize,
        content: &crate::content::ContentCatalog,
    ) -> io::Result<Self> {
        if !(1..=32).contains(&section_count) || take(&mut bytes, 1)?[0] as usize != section_count {
            return Err(invalid());
        }
        let mut states = Vec::with_capacity(section_count);
        let mut light = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            states.push(decode_section(&mut bytes, |bytes| {
                T::from_id(
                    u32::from_le_bytes(take(bytes, 4)?.try_into().unwrap()),
                    content,
                )
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
