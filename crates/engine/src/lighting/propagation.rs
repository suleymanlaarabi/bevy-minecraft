use super::snapshots::{LightCell, SnapshotAccess};
use hollow_core::{VoxelCell, content::StateDefinition};
use std::collections::VecDeque;

#[derive(Default)]
pub enum LightingMode {
    #[default]
    Default,
    Full,
    Dark,
}

#[derive(Default)]
pub struct SnapshotLightEngine {
    block_increase: VecDeque<LightCell>,
    block_decrease: VecDeque<(LightCell, u8)>,
    block_pending: std::collections::BTreeMap<(u32, u32), Box<[u64; 64]>>,
    sky_decrease: VecDeque<(LightCell, u8)>,
    sky_increase: VecDeque<LightCell>,
    sky_seeds: Vec<(LightCell, u8)>,
}

/// Direct sky stays at 15 downward through transparent, non-occluding cells.
fn propagated_sky(state: &StateDefinition, light: u8, downward: bool) -> u8 {
    let opacity = state.opacity.max(u8::from(state.can_occlude()));
    if light == 15 && downward && opacity == 0 && !state.can_occlude() {
        15
    } else {
        light.saturating_sub(1).saturating_sub(opacity)
    }
}

impl SnapshotLightEngine {
    pub(super) fn release_excess(&mut self) {
        // Keep ordinary edit buffers, but release capacity retained by large cascades.
        self.block_increase.shrink_to(4096);
        self.block_decrease.shrink_to(4096);
    }
    pub(super) fn queue_block_light_increase(&mut self, pos: LightCell, _light: u8) {
        // Only queued sections allocate a bitset; distant edit islands need no dense table.
        let bits = self
            .block_pending
            .entry((pos.chunk, pos.index / 4096))
            .or_insert_with(|| Box::new([0; 64]));
        let (word, bit) = ((pos.index as usize % 4096) / 64, pos.index % 64);
        if bits[word] & (1 << bit) == 0 {
            bits[word] |= 1 << bit;
            self.block_increase.push_back(pos);
        }
    }
    pub(super) fn invalidate_block<T: VoxelCell>(
        &mut self,
        level: &mut SnapshotAccess<T>,
        pos: LightCell,
    ) {
        let old = level.light(pos) & 15;
        level.set_light(pos, 0, 15);
        self.block_decrease.push_back((pos, old));
        self.queue_block_light_increase(pos, 0);
        for side in 0..6 {
            if let Some(neighbor) = level.neighbor(pos, side) {
                self.queue_block_light_increase(neighbor, 0);
            }
        }
    }

    pub(super) fn queue_sky_light_increase(&mut self, pos: LightCell, _light: u8) {
        self.sky_increase.push_back(pos);
    }

    pub(super) fn invalidate_sky_column<T: VoxelCell>(
        &mut self,
        level: &mut SnapshotAccess<T>,
        chunk: u32,
        xz: u32,
        height: u32,
    ) {
        let mut direct = 15;
        for y in (0..height).rev() {
            let pos = LightCell {
                chunk,
                index: y * 256 + xz,
            };
            let old = level.light(pos) >> 4;
            if old > 0 {
                level.set_light(pos, 0, 240);
                self.sky_decrease.push_back((pos, old));
            }
            direct = match level.lighting_config {
                LightingMode::Full => 15,
                LightingMode::Dark => 0,
                LightingMode::Default if y + 1 == height => sky_source(level.definition(pos)),
                LightingMode::Default => propagated_sky(level.definition(pos), direct, true),
            };
            if direct > 0 {
                self.sky_seeds.push((pos, direct));
            }
            for side in 2..6 {
                if let Some(neighbor) = level.neighbor(pos, side) {
                    self.sky_increase.push_back(neighbor);
                }
            }
        }
    }

    pub(super) fn perform_block_light_updates<T: VoxelCell>(
        &mut self,
        level: &mut SnapshotAccess<T>,
    ) {
        while let Some((pos, removed)) = self.block_decrease.pop_front() {
            for side in 0..6 {
                let Some(neighbor) = level.neighbor(pos, side) else {
                    continue;
                };
                let light = level.light(neighbor) & 15;
                if light > 0 && light < removed {
                    level.set_light(neighbor, 0, 15);
                    self.block_decrease.push_back((neighbor, light));
                    if level.definition(neighbor).luminance > 0 {
                        self.queue_block_light_increase(neighbor, 0);
                    }
                } else if light > 0 {
                    self.queue_block_light_increase(neighbor, 0);
                }
            }
        }
        while let Some(pos) = self.block_increase.pop_front() {
            self.block_pending
                .get_mut(&(pos.chunk, pos.index / 4096))
                .unwrap()[(pos.index as usize % 4096) / 64] &= !(1 << (pos.index % 64));
            let light = (level.light(pos) & 15).max(level.definition(pos).luminance);
            level.set_light(pos, light, 15);
            if light <= 1 {
                continue;
            }
            for side in 0..6 {
                let Some(neighbor) = level.neighbor(pos, side) else {
                    continue;
                };
                let next = light.saturating_sub(level.definition(neighbor).opacity.max(1));
                if next > (level.light(neighbor) & 15) {
                    level.set_light(neighbor, next, 15);
                    if next > 1 {
                        self.queue_block_light_increase(neighbor, next);
                    }
                }
            }
        }
        self.block_pending.clear();
    }

    pub(super) fn perform_sky_light_updates<T: VoxelCell>(
        &mut self,
        level: &mut SnapshotAccess<T>,
    ) {
        while let Some((pos, removed)) = self.sky_decrease.pop_front() {
            for side in 0..6 {
                let Some(neighbor) = level.neighbor(pos, side) else {
                    continue;
                };
                let light = level.light(neighbor) >> 4;
                if light == 0 {
                    continue;
                }
                let expected = propagated_sky(level.definition(neighbor), removed, side == 0);
                if light == expected || light < removed {
                    level.set_light(neighbor, 0, 240);
                    self.sky_decrease.push_back((neighbor, light));
                } else {
                    self.sky_increase.push_back(neighbor);
                }
            }
        }
        for (pos, light) in self.sky_seeds.drain(..) {
            level.set_light(pos, light << 4, 240);
            self.sky_increase.push_back(pos);
        }
        while let Some(pos) = self.sky_increase.pop_front() {
            let light = level.light(pos) >> 4;
            if light == 0 {
                continue;
            }
            for side in 0..6 {
                // Missing neighbors are rejected here before any read/write,
                // preventing endless re-queuing at an unloaded border.
                let Some(neighbor) = level.neighbor(pos, side) else {
                    continue;
                };
                let next = propagated_sky(level.definition(neighbor), light, side == 0);
                if next > level.light(neighbor) >> 4 {
                    level.set_light(neighbor, next << 4, 240);
                    self.sky_increase.push_back(neighbor);
                }
            }
        }
    }

    /// Seed a reset column at the top world boundary; downward propagation handles the rest.
    pub(super) fn seed_sky<T: VoxelCell>(&mut self, level: &mut SnapshotAccess<T>, pos: LightCell) {
        let expected = match level.lighting_config {
            LightingMode::Full => {
                level.set_light(pos, 240, 240);
                return;
            }
            LightingMode::Dark => {
                level.set_light(pos, 0, 240);
                return;
            }
            LightingMode::Default => sky_source(level.definition(pos)),
        };
        level.set_light(pos, expected << 4, 240);
        if expected > 0 {
            self.sky_increase.push_back(pos);
        }
    }
}

fn sky_source(state: &StateDefinition) -> u8 {
    let opacity = state.opacity.max(u8::from(state.can_occlude()));
    if opacity == 15 || state.is_solid_render() {
        0
    } else {
        15u8.saturating_sub(opacity)
    }
}
