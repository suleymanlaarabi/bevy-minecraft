//! Lighting accesses working buffers and completed shared sections in one cache.
use crate::{
    chunk_system::{Cache, Chunk},
    generation::height_limit::HeightLimitView,
};
use pumpkin_util::math::position::BlockPos;

fn address(cache: &Cache, pos: BlockPos) -> Option<(usize, usize)> {
    let (x, z) = ((pos.0.x >> 4) - cache.x, (pos.0.z >> 4) - cache.z);
    let y = pos.0.y - cache.bottom_y() as i32;
    if x < 0 || x >= cache.size || z < 0 || z >= cache.size || y < 0 || y >= cache.height() as i32 {
        return None;
    }
    Some((
        (x * cache.size + z) as usize,
        y as usize * 256 + (pos.0.z & 15) as usize * 16 + (pos.0.x & 15) as usize,
    ))
}
fn get(cache: &Cache, pos: BlockPos, sky: bool) -> u8 {
    let Some((chunk, index)) = address(cache, pos) else {
        return 0;
    };
    match &cache.chunks[chunk] {
        Chunk::Complete(data) => {
            let value = data.light.lock().unwrap().get(index);
            if sky { value >> 4 } else { value & 15 }
        }
        Chunk::Proto(data) => {
            let sections = if sky {
                &data.light.sky_light
            } else {
                &data.light.block_light
            };
            sections.get(index / 4096).map_or(0, |section| {
                section.get(index & 15, (index >> 8) & 15, (index >> 4) & 15)
            })
        }
    }
}
fn set(cache: &mut Cache, pos: BlockPos, value: u8, sky: bool) {
    let Some((chunk, index)) = address(cache, pos) else {
        return;
    };
    match &mut cache.chunks[chunk] {
        Chunk::Complete(data) => {
            set_shared_channel(&mut data.light.lock().unwrap(), index, value, sky)
        }
        Chunk::Proto(data) => {
            let sections = if sky {
                &mut data.light.sky_light
            } else {
                &mut data.light.block_light
            };
            if let Some(section) = sections.get_mut(index / 4096) {
                section.set(index & 15, (index >> 8) & 15, (index >> 4) & 15, value);
            }
        }
    }
}
pub(crate) fn set_shared_channel(
    light: &mut crate::sections::Layer<u8>,
    index: usize,
    value: u8,
    sky: bool,
) {
    let old = light.get(index);
    light.set(
        index,
        if sky {
            (old & 15) | (value << 4)
        } else {
            (old & 240) | value
        },
    );
}
pub fn get_block_light(cache: &Cache, pos: BlockPos) -> u8 {
    get(cache, pos, false)
}
pub fn get_sky_light(cache: &Cache, pos: BlockPos) -> u8 {
    get(cache, pos, true)
}
pub fn set_block_light(cache: &mut Cache, pos: BlockPos, value: u8) {
    set(cache, pos, value, false);
}
pub fn set_sky_light(cache: &mut Cache, pos: BlockPos, value: u8) {
    set(cache, pos, value, true);
}
