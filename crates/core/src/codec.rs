//! Compact terrain records over shared sections, independent of game content.
use crate::{
    VoxelCell,
    coordinates::SECTION_VOLUME,
    sections::{Layer, Section, SectionValue},
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
fn section_bytes<T>(section: &Section<T>, width: usize) -> usize {
    1 + match section {
        Section::Uniform(_) => width,
        Section::Narrow { .. } => 4 + 2 * SECTION_VOLUME,
        Section::Dense(_) => width * SECTION_VOLUME,
    }
}
fn encode_section<T: SectionValue>(section: &Section<T>, width: usize, bytes: &mut Vec<u8>) {
    let start = bytes.len();
    bytes.resize(start + section_bytes(section, width), 0);
    let out = &mut bytes[start..];
    match section {
        Section::Uniform(value) => out[1..].copy_from_slice(&value.bits().to_le_bytes()[..width]),
        Section::Narrow { prefix, cells } => {
            out[0] = 2;
            out[1..5].copy_from_slice(&prefix.to_le_bytes());
            for (out, cell) in out[5..].chunks_exact_mut(2).zip(cells.iter()) {
                out.copy_from_slice(&cell.to_le_bytes());
            }
        }
        Section::Dense(cells) => {
            out[0] = 1;
            for (out, cell) in out[1..].chunks_exact_mut(width).zip(cells.iter()) {
                out.copy_from_slice(&cell.bits().to_le_bytes()[..width]);
            }
        }
    }
}
fn decode_section<T: SectionValue>(
    bytes: &mut &[u8],
    width: usize,
    decode: impl Fn(u32) -> io::Result<T>,
) -> io::Result<Section<T>> {
    let read = |bytes: &mut &[u8]| -> io::Result<T> {
        let mut bits = [0; 4];
        bits[..width].copy_from_slice(take(bytes, width)?);
        decode(u32::from_le_bytes(bits))
    };
    match take(bytes, 1)?[0] {
        0 => Ok(Section::Uniform(read(bytes)?)),
        1 => {
            let mut cells = Arc::new([read(bytes)?; SECTION_VOLUME]);
            for cell in &mut Arc::get_mut(&mut cells).unwrap()[1..] {
                *cell = read(bytes)?;
            }
            Ok(Section::Dense(cells))
        }
        2 if T::NARROW => {
            let prefix = u32::from_le_bytes(take(bytes, 4)?.try_into().unwrap());
            if prefix & 0xffff != 0 {
                return Err(invalid());
            }
            let mut cells = Arc::new([0; SECTION_VOLUME]);
            for cell in Arc::get_mut(&mut cells).unwrap() {
                *cell = u16::from_le_bytes(take(bytes, 2)?.try_into().unwrap());
                decode(prefix | u32::from(*cell))?;
            }
            Ok(Section::Narrow { prefix, cells })
        }
        _ => Err(invalid()),
    }
}
impl<T: VoxelCell> TerrainRecord<T> {
    pub fn encode(states: &Layer<T>, light: &Layer<u8>, modified: bool, bytes: &mut Vec<u8>) {
        let count = states.section_count();
        bytes.reserve_exact(
            2 + (0..count)
                .map(|i| section_bytes(states.section(i), 4) + section_bytes(light.section(i), 1))
                .sum::<usize>(),
        );
        bytes.push(u8::try_from(count).expect("Section count exceeds terrain storage"));
        for i in 0..count {
            encode_section(states.section(i), 4, bytes);
            encode_section(light.section(i), 1, bytes);
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
            states.push(decode_section(&mut bytes, 4, |bits| {
                T::from_id(bits, content).ok_or_else(invalid)
            })?);
            light.push(decode_section(&mut bytes, 1, |bits| Ok(bits as u8))?);
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
