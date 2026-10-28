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
    block_increase: VecDeque<(LightCell, u8)>,
    sky_decrease: VecDeque<(LightCell, u8)>,
    sky_increase: VecDeque<(LightCell, u8)>,
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
    pub fn queue_block_light_increase(&mut self, pos: LightCell, light: u8) {
        self.block_increase.push_back((pos, light));
    }

    pub fn queue_sky_light_increase(&mut self, pos: LightCell, light: u8) {
        self.sky_increase.push_back((pos, light));
    }

    pub fn perform_block_light_updates<T: VoxelCell>(&mut self, level: &mut SnapshotAccess<T>) {
        // Reconciliation resets block light before seeding all surviving emitters;
        // unlike edit-frame lighting, it has no block-light decrease work.
        while let Some((pos, light)) = self.block_increase.pop_front() {
            for side in 0..6 {
                let Some(neighbor) = level.neighbor(pos, side) else {
                    continue;
                };
                let next = light.saturating_sub(level.definition(neighbor).opacity.max(1));
                if next > (level.light(neighbor) & 15) {
                    level.set_light(neighbor, next, 15);
                    if next > 1 {
                        self.block_increase.push_back((neighbor, next));
                    }
                }
            }
        }
    }

    pub fn perform_sky_light_updates<T: VoxelCell>(&mut self, level: &mut SnapshotAccess<T>) {
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
                } else if light > removed {
                    self.sky_increase.push_back((neighbor, light));
                }
            }
        }
        while let Some((pos, light)) = self.sky_increase.pop_front() {
            for side in 0..6 {
                // Missing neighbors are rejected here before any read/write,
                // preventing endless re-queuing at an unloaded border.
                let Some(neighbor) = level.neighbor(pos, side) else {
                    continue;
                };
                let next = propagated_sky(level.definition(neighbor), light, side == 0);
                if next > level.light(neighbor) >> 4 {
                    level.set_light(neighbor, next << 4, 240);
                    self.sky_increase.push_back((neighbor, next));
                }
            }
        }
    }

    pub fn check_sky_light_updates<T: VoxelCell>(
        &mut self,
        level: &mut SnapshotAccess<T>,
        pos: LightCell,
    ) {
        match level.lighting_config {
            LightingMode::Full => {
                level.set_light(pos, 240, 240);
                return;
            }
            LightingMode::Dark => {
                level.set_light(pos, 0, 240);
                return;
            }
            LightingMode::Default => {}
        }
        let current = level.light(pos) >> 4;
        let state = level.definition(pos);
        let occludes = state.can_occlude();
        let opacity = state.opacity.max(u8::from(occludes));
        let expected = if opacity == 15 || state.is_solid_render() {
            0
        } else {
            let mut above = pos;
            let mut open = true;
            while let Some(next) = level.neighbor(above, 1) {
                above = next;
                let state = level.definition(above);
                if state.can_occlude() || state.opacity > 0 {
                    open = false;
                    break;
                }
            }
            if open {
                15u8.saturating_sub(opacity)
            } else {
                (0..6)
                    .filter_map(|side| {
                        let light = level.light(level.neighbor(pos, side)?) >> 4;
                        Some(if light == 15 && side == 1 && opacity == 0 && !occludes {
                            15
                        } else {
                            light.saturating_sub(1)
                        })
                    })
                    .max()
                    .unwrap_or(0)
                    .saturating_sub(opacity)
            }
        };
        if expected != current {
            level.set_light(pos, expected << 4, 240);
        }
        if expected < current {
            self.sky_decrease.push_back((pos, current));
        } else if expected > 0 {
            // Seed once, including unchanged bright cells; the previous path
            // queued an increased source twice.
            self.sky_increase.push_back((pos, expected));
        }
    }
}
