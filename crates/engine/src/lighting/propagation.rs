use super::snapshots::SnapshotAccess;
use bevy_math::IVec3;
use hollow_core::VoxelCell;
use std::marker::PhantomData;

#[derive(Default)]
pub enum LightingMode {
    #[default]
    Default,
    Full,
    Dark,
}
const SIDES: [IVec3; 6] = [
    IVec3::NEG_Y,
    IVec3::Y,
    IVec3::NEG_Z,
    IVec3::Z,
    IVec3::NEG_X,
    IVec3::X,
];
use std::collections::VecDeque;

pub struct SnapshotLightEngine<T: VoxelCell> {
    cell: PhantomData<T>,
    min_y: i32,
    max_y: i32,
    block_decrease: VecDeque<(IVec3, u8)>,
    block_increase: VecDeque<(IVec3, u8)>,
    sky_decrease: VecDeque<(IVec3, u8)>,
    sky_increase: VecDeque<(IVec3, u8)>,
}

impl<T: VoxelCell> SnapshotLightEngine<T> {
    #[must_use]
    pub const fn new(min_y: i32, max_y: i32) -> Self {
        Self {
            cell: PhantomData,
            min_y,
            max_y,
            block_decrease: VecDeque::new(),
            block_increase: VecDeque::new(),
            sky_decrease: VecDeque::new(),
            sky_increase: VecDeque::new(),
        }
    }
}

impl<T: VoxelCell> SnapshotLightEngine<T> {
    /// Checks if there is an open sky above the given position (no opaque blocks blocking sky light).
    fn has_open_sky_above(&mut self, level: &mut SnapshotAccess<T>, pos: &IVec3) -> bool {
        let max_y = self.max_y - 1;
        let mut current_pos = *pos;

        // Scan upward until we hit sky or an opaque block
        while current_pos.y < max_y {
            current_pos.y += 1;

            let state = level.definition(&current_pos);
            if state.can_occlude() || state.opacity > 0 {
                return false; // Hit an opaque or occluding block before reaching sky
            }
        }

        true // Reached sky without hitting opaque blocks
    }

    pub fn queue_block_light_decrease(&mut self, pos: IVec3, level: u8) {
        self.block_decrease.push_back((pos, level));
    }

    pub fn queue_block_light_increase(&mut self, pos: IVec3, level: u8) {
        self.block_increase.push_back((pos, level));
    }

    pub fn queue_sky_light_decrease(&mut self, pos: IVec3, level: u8) {
        self.sky_decrease.push_back((pos, level));
    }

    pub fn queue_sky_light_increase(&mut self, pos: IVec3, level: u8) {
        self.sky_increase.push_back((pos, level));
    }

    pub fn perform_block_light_updates(&mut self, level: &mut SnapshotAccess<T>) -> i32 {
        let mut updates = 0;

        // Keep processing until both queues are empty
        // Light propagation queues new updates, so we need to process until convergence
        loop {
            let decrease_updates = self.perform_block_light_decrease_updates(level);
            let increase_updates = self.perform_block_light_increase_updates(level);

            updates += decrease_updates + increase_updates;

            // Stop when no more updates were processed
            if decrease_updates == 0 && increase_updates == 0 {
                break;
            }
        }

        updates
    }

    fn perform_block_light_decrease_updates(&mut self, level: &mut SnapshotAccess<T>) -> i32 {
        let mut updates = 0;

        while let Some((pos, expected_light)) = self.block_decrease.pop_front() {
            self.propagate_block_light_decrease(level, &pos, expected_light);
            updates += 1;
        }

        updates
    }

    fn perform_block_light_increase_updates(&mut self, level: &mut SnapshotAccess<T>) -> i32 {
        let mut updates = 0;

        while let Some((pos, expected_light)) = self.block_increase.pop_front() {
            self.propagate_block_light_increase(level, &pos, expected_light);
            updates += 1;
        }

        updates
    }

    fn propagate_block_light_increase(
        &mut self,
        level: &mut SnapshotAccess<T>,
        pos: &IVec3,
        light_level: u8,
    ) {
        for dir in SIDES {
            let neighbor_pos = *pos + dir;

            if neighbor_pos.y < self.min_y || neighbor_pos.y >= self.max_y {
                continue;
            }

            if let Some(neighbor_light) = self.get_block_light_level(level, &neighbor_pos) {
                let neighbor_state = level.definition(&neighbor_pos);
                let opacity = neighbor_state.opacity.max(1);
                let new_light = light_level.saturating_sub(opacity);

                // Only propagate if new light is brighter than current light
                if new_light > neighbor_light
                    && self
                        .set_block_light_level(level, &neighbor_pos, new_light)
                        .is_ok()
                    && new_light > 1
                {
                    self.queue_block_light_increase(neighbor_pos, new_light);
                }
            }
        }
    }

    fn propagate_block_light_decrease(
        &mut self,
        level: &mut SnapshotAccess<T>,
        pos: &IVec3,
        removed_light_level: u8,
    ) {
        // Check what the current light level actually is at this position
        let current_level = self.get_block_light_level(level, pos).unwrap_or(0);

        // Only propagate decrease if this position hasn't already been reset to 0
        // This prevents positions that were intentionally set to 0 from propagating light
        if current_level == 0 && removed_light_level > 0 {
            // This position was already darkened, so we propagate the darkness to neighbors
            for dir in SIDES {
                let neighbor_pos = *pos + dir;

                if neighbor_pos.y < self.min_y || neighbor_pos.y >= self.max_y {
                    continue;
                }

                if let Some(neighbor_light) = self.get_block_light_level(level, &neighbor_pos) {
                    if neighbor_light == 0 {
                        continue; // Skip if already 0
                    }

                    let neighbor_state = level.definition(&neighbor_pos);
                    let opacity = neighbor_state.opacity.max(1);

                    let expected_from_removed_source = removed_light_level.saturating_sub(opacity);

                    if neighbor_light <= expected_from_removed_source {
                        let neighbor_luminance = neighbor_state.luminance;

                        if neighbor_luminance == 0 {
                            // No self-emission, darken it completely and continue propagation
                            self.set_block_light_level(level, &neighbor_pos, 0).ok();
                            self.queue_block_light_decrease(neighbor_pos, neighbor_light);
                        } else {
                            // Has self-emission, set to its own light and re-propagate from it
                            self.set_block_light_level(level, &neighbor_pos, neighbor_luminance)
                                .ok();
                            self.queue_block_light_increase(neighbor_pos, neighbor_luminance);
                        }
                    } else {
                        // This neighbor has brighter light from another source, re-propagate from it
                        self.queue_block_light_increase(neighbor_pos, neighbor_light);
                    }
                }
            }
        }
    }

    pub fn perform_sky_light_updates(&mut self, level: &mut SnapshotAccess<T>) -> i32 {
        let mut updates = 0;
        loop {
            let decrease_updates = self.perform_sky_light_decrease_updates(level);
            let increase_updates = self.perform_sky_light_increase_updates(level);

            updates += decrease_updates + increase_updates;

            if decrease_updates == 0 && increase_updates == 0 {
                break;
            }
        }
        updates
    }

    fn perform_sky_light_decrease_updates(&mut self, level: &mut SnapshotAccess<T>) -> i32 {
        let mut updates = 0;
        while let Some((pos, expected_light)) = self.sky_decrease.pop_front() {
            self.propagate_sky_light_decrease(level, &pos, expected_light);
            updates += 1;
        }
        updates
    }

    fn perform_sky_light_increase_updates(&mut self, level: &mut SnapshotAccess<T>) -> i32 {
        let mut updates = 0;
        while let Some((pos, expected_light)) = self.sky_increase.pop_front() {
            self.propagate_sky_light_increase(level, &pos, expected_light);
            updates += 1;
        }
        updates
    }

    fn propagate_sky_light_increase(
        &mut self,
        level: &mut SnapshotAccess<T>,
        pos: &IVec3,
        light_level: u8,
    ) {
        for dir in SIDES {
            let neighbor_pos = *pos + dir;

            if neighbor_pos.y < self.min_y || neighbor_pos.y >= self.max_y {
                continue;
            }

            // Never propagate into an unloaded chunk. Writes to an unloaded
            // chunk are dropped silently, so the "brighter than neighbor" check
            // below would stay true forever and keep re-queuing the same
            // position, spinning this loop indefinitely at the border between
            // loaded and unloaded chunks.
            let (neighbor_chunk, _) = hollow_core::coordinates::split_world_position(neighbor_pos);
            if !level.is_chunk_loaded(&neighbor_chunk) {
                continue;
            }

            let neighbor_light = self.get_sky_light_level(level, &neighbor_pos);
            let neighbor_state = level.definition(&neighbor_pos);
            let opacity = if neighbor_state.can_occlude() {
                neighbor_state.opacity.max(1)
            } else {
                neighbor_state.opacity
            };

            // Calculate new light level for neighbor
            let new_light = if light_level == 15
                && dir == IVec3::NEG_Y
                && opacity == 0
                && !neighbor_state.can_occlude()
            {
                // Special case: Sky light at 15 propagates down as 15 through transparent blocks
                15
            } else {
                // Normal propagation: reduce by 1 for distance, then by opacity
                light_level.saturating_sub(1).saturating_sub(opacity)
            };

            // Only propagate if new light is brighter than current light
            if new_light > neighbor_light {
                self.set_sky_light_level(level, &neighbor_pos, new_light)
                    .ok();

                if new_light > 0 {
                    self.queue_sky_light_increase(neighbor_pos, new_light);
                }
            }
        }
    }

    fn propagate_sky_light_decrease(
        &mut self,
        level: &mut SnapshotAccess<T>,
        pos: &IVec3,
        removed_light: u8,
    ) {
        for dir in SIDES {
            let neighbor_pos = *pos + dir;

            if neighbor_pos.y < self.min_y || neighbor_pos.y >= self.max_y {
                continue;
            }

            // See `propagate_sky_light_increase`: skip unloaded chunks so sky
            // light updates never spin at loaded/unloaded chunk borders.
            let (neighbor_chunk, _) = hollow_core::coordinates::split_world_position(neighbor_pos);
            if !level.is_chunk_loaded(&neighbor_chunk) {
                continue;
            }

            let neighbor_light = self.get_sky_light_level(level, &neighbor_pos);
            if neighbor_light == 0 {
                continue; // Already dark
            }

            let neighbor_state = level.definition(&neighbor_pos);
            let opacity = if neighbor_state.can_occlude() {
                neighbor_state.opacity.max(1)
            } else {
                neighbor_state.opacity
            };

            // Calculate what we would have given this neighbor
            let expected = if removed_light == 15
                && dir == IVec3::NEG_Y
                && opacity == 0
                && !neighbor_state.can_occlude()
            {
                15
            } else {
                removed_light.saturating_sub(1).saturating_sub(opacity)
            };

            if neighbor_light == expected || neighbor_light < removed_light {
                // This neighbor was lit by us, darken it
                self.set_sky_light_level(level, &neighbor_pos, 0).ok();
                self.queue_sky_light_decrease(neighbor_pos, neighbor_light);
            } else if neighbor_light > removed_light {
                // Neighbor has brighter light from another source
                // Re-propagate from it to fill in the gap we left
                self.queue_sky_light_increase(neighbor_pos, neighbor_light);
            }
        }
    }

    pub fn check_sky_light_updates(&mut self, level: &mut SnapshotAccess<T>, pos: IVec3) {
        match level.lighting_config {
            LightingMode::Full => {
                self.set_sky_light_level(level, &pos, 15).ok();
                return;
            }
            LightingMode::Dark => {
                self.set_sky_light_level(level, &pos, 0).ok();
                return;
            }
            LightingMode::Default => {}
        }

        let current_light = self.get_sky_light_level(level, &pos);
        let block_state = level.definition(&pos);
        let occludes = block_state.can_occlude();
        let solid_render = block_state.is_solid_render();
        let opacity = if occludes {
            block_state.opacity.max(1)
        } else {
            block_state.opacity
        };

        // Calculate expected sky light
        let expected_light = if opacity == 15 || solid_render {
            // Fully opaque block = no light
            0
        } else {
            // Check if there's open sky above
            let has_sky = self.has_open_sky_above(level, &pos);

            if has_sky {
                // Direct sunlight, reduced by opacity
                15u8.saturating_sub(opacity)
            } else {
                // No direct sky, check neighbors for best light
                let mut best_light = 0;

                for dir in SIDES {
                    let neighbor_pos = pos + dir;

                    if neighbor_pos.y < self.min_y || neighbor_pos.y >= self.max_y {
                        continue;
                    }

                    let neighbor_light = self.get_sky_light_level(level, &neighbor_pos);
                    // Calculate potential light from this neighbor
                    let potential =
                        if neighbor_light == 15 && dir == IVec3::Y && opacity == 0 && !occludes {
                            // Sky light at 15 from above stays 15 through transparent non-occluding blocks
                            15
                        } else {
                            // Normal decay
                            neighbor_light.saturating_sub(1)
                        };

                    best_light = best_light.max(potential);
                }

                // Apply opacity to the best incoming light
                best_light.saturating_sub(opacity)
            }
        };

        // Update if needed
        if expected_light < current_light {
            // Light decreased
            self.set_sky_light_level(level, &pos, expected_light).ok();
            self.queue_sky_light_decrease(pos, current_light);
        } else if expected_light > current_light {
            // Light increased
            self.set_sky_light_level(level, &pos, expected_light).ok();
            self.queue_sky_light_increase(pos, expected_light);
        }

        // Notify neighbors if light increased or stayed same
        if expected_light >= current_light {
            self.check_neighbors_sky_light_updates(pos, expected_light);
        }
    }

    pub fn check_neighbors_sky_light_updates(&mut self, pos: IVec3, current_light: u8) {
        // When we update a position, propagate to neighbors
        if current_light > 0 {
            self.queue_sky_light_increase(pos, current_light);
        }
    }

    pub fn get_block_light_level(
        &mut self,
        level: &mut SnapshotAccess<T>,
        position: &IVec3,
    ) -> Option<u8> {
        level.light(position).map(|light| light & 15)
    }
    pub fn get_sky_light_level(&mut self, level: &mut SnapshotAccess<T>, position: &IVec3) -> u8 {
        level.light(position).map_or(0, |light| light >> 4)
    }
    pub fn set_block_light_level(
        &mut self,
        level: &mut SnapshotAccess<T>,
        position: &IVec3,
        light: u8,
    ) -> Result<(), String> {
        level.set_light(position, light, 15)
    }
    pub fn set_sky_light_level(
        &mut self,
        level: &mut SnapshotAccess<T>,
        position: &IVec3,
        light: u8,
    ) -> Result<(), String> {
        level.set_light(position, light << 4, 240)
    }
}
