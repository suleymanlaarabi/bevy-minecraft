use crate::chunk::ChunkLight;
pub(crate) fn working_sections(light: &ChunkLight, count: usize) -> crate::sections::Layer<u8> {
    use crate::chunk::format::LightContainer::{Empty, Full};
    use crate::sections::{Layer, Section};
    Layer::from_sections((0..count).map(|section| {
        let sky = light.sky_light.get(section).unwrap_or(&Empty(0));
        let block = light.block_light.get(section).unwrap_or(&Empty(0));
        if let (Empty(sky), Empty(block)) = (sky, block) {
            return Section::Uniform((sky << 4) | block);
        }
        // Nibble containers use the section cell order: unpack one byte into
        // two cells instead of addressing 4096 nibbles individually.
        let mut cells = [0u8; 4096];
        match sky {
            Empty(value) => cells.fill(value << 4),
            Full(data) => {
                for (pair, &byte) in cells.chunks_exact_mut(2).zip(data.iter()) {
                    pair[0] = byte << 4;
                    pair[1] = byte & 0xF0;
                }
            }
        }
        match block {
            Empty(value) => cells.iter_mut().for_each(|cell| *cell |= value),
            Full(data) => {
                for (pair, &byte) in cells.chunks_exact_mut(2).zip(data.iter()) {
                    pair[0] |= byte & 0x0F;
                    pair[1] |= byte >> 4;
                }
            }
        }
        Section::from_cells(&cells)
    }))
}
