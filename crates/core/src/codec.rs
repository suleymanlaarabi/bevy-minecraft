//! Compact terrain records over shared sections, independent of game content.
use crate::{
    VoxelCell,
    coordinates::SECTION_VOLUME,
    sections::{Layer, Palette, Section, SectionValue},
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
        Section::Palette4(p) => 2 + usize::from(p.len) * width + p.indices.len(),
        Section::Palette8(p) => 2 + usize::from(p.len) * width + p.indices.len(),
    }
}
fn encode_section<T: SectionValue>(section: &Section<T>, width: usize, bytes: &mut Vec<u8>) {
    let start = bytes.len();
    bytes.resize(start + section_bytes(section, width), 0);
    let out = &mut bytes[start..];
    match section {
        Section::Palette4(p) => encode_palette(p, width, 3, out),
        Section::Palette8(p) => encode_palette(p, width, 4, out),
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
fn encode_palette<T: SectionValue, const N: usize, const BYTES: usize>(
    palette: &Palette<T, N, BYTES>,
    width: usize,
    tag: u8,
    out: &mut [u8],
) {
    out[0] = tag;
    out[1..3].copy_from_slice(&palette.len.to_le_bytes());
    let end = 3 + usize::from(palette.len) * width;
    for (out, value) in out[3..end].chunks_exact_mut(width).zip(&palette.values) {
        out.copy_from_slice(&value.bits().to_le_bytes()[..width]);
    }
    out[end..].copy_from_slice(&palette.indices);
}
fn decode_palette<T: SectionValue, const N: usize, const BYTES: usize>(
    bytes: &mut &[u8],
    read: impl Fn(&mut &[u8]) -> io::Result<T>,
) -> io::Result<Arc<Palette<T, N, BYTES>>> {
    let len = u16::from_le_bytes(take(bytes, 2)?.try_into().unwrap());
    if len == 0 || usize::from(len) > N {
        return Err(invalid());
    }
    let mut values = [read(bytes)?; N];
    for value in &mut values[1..usize::from(len)] {
        *value = read(bytes)?;
    }
    let indices: [u8; BYTES] = take(bytes, BYTES)?.try_into().unwrap();
    let mut used = [false; N];
    for &byte in &indices {
        let first = usize::from(if N == 16 { byte & 15 } else { byte });
        if first >= usize::from(len) {
            return Err(invalid());
        }
        used[first] = true;
        if N == 16 {
            let second = usize::from(byte >> 4);
            if second >= usize::from(len) {
                return Err(invalid());
            }
            used[second] = true;
        }
    }
    // Section predicates scan the palette, so every declared entry must occur.
    if used[..usize::from(len)].contains(&false) {
        return Err(invalid());
    }
    Ok(Arc::new(Palette {
        values,
        len,
        indices,
    }))
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
        3 => Ok(Section::Palette4(decode_palette(bytes, read)?)),
        4 => Ok(Section::Palette8(decode_palette(bytes, read)?)),
        1 => {
            let mut cells = [read(bytes)?; SECTION_VOLUME];
            for cell in &mut cells[1..] {
                *cell = read(bytes)?;
            }
            Ok(Section::from_cells(&cells))
        }
        2 if T::NARROW => {
            let prefix = u32::from_le_bytes(take(bytes, 4)?.try_into().unwrap());
            if prefix & 0xffff != 0 {
                return Err(invalid());
            }
            let first = u16::from_le_bytes(take(bytes, 2)?.try_into().unwrap());
            let mut cells = [decode(prefix | u32::from(first))?; SECTION_VOLUME];
            for cell in &mut cells[1..] {
                let low = u16::from_le_bytes(take(bytes, 2)?.try_into().unwrap());
                *cell = decode(prefix | u32::from(low))?;
            }
            Ok(Section::from_cells(&cells))
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
