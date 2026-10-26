use crate::chunk::ChunkLight;
pub(crate) fn working_sections(light: &ChunkLight, count: usize) -> crate::sections::Layer<u8> {
    use crate::chunk::format::LightContainer::Empty;
    use crate::sections::{Layer, Section};
    Layer::from_sections((0..count).map(|section| {
        let sky = light.sky_light.get(section).unwrap_or(&Empty(0));
        let block = light.block_light.get(section).unwrap_or(&Empty(0));
        match (sky, block) {
            (Empty(sky), Empty(block)) => Section::Uniform((sky << 4) | block),
            _ => Section::from_cells(&std::array::from_fn::<_, 4096, _>(|i| {
                let (x, y, z) = (i & 15, i >> 8, (i >> 4) & 15);
                (sky.get(x, y, z) << 4) | block.get(x, y, z)
            })),
        }
    }))
}
