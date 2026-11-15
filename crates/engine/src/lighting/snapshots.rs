//! Edit lighting over shared sections, using the generation engine's propagation rules.
use super::propagation::{LightingMode, SnapshotLightEngine};
use bevy_math::IVec3;
use hollow_core::{
    StateId, VoxelCell,
    content::{ContentCatalog, StateDefinition},
    coordinates::SECTION_VOLUME,
    sections::Layer,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct LightingChunk<T: VoxelCell> {
    pub x: i32,
    pub z: i32,
    pub states: Layer<T>,
    pub light: Layer<u8>,
    pub modified: bool,
    /// Complete light-relevant edits since the previous input, or a full section diff.
    pub changed_cells: Option<Vec<u32>>,
}
pub struct LightingUpdate {
    pub x: i32,
    pub z: i32,
    pub light: Layer<u8>,
}

/// Previous inputs retain the same shared allocations as the game.
pub struct LightingSnapshots<T: VoxelCell> {
    access: SnapshotAccess<T>,
}

/// Stable only during one reconciliation; queues drain before slots can be reused.
#[derive(Clone, Copy)]
pub(super) struct LightCell {
    pub(super) chunk: u32,
    pub(super) index: u32,
}

struct LightingSlot<T: VoxelCell> {
    chunk: Option<LightingChunk<T>>,
    neighbors: [Option<u32>; 4],
    changed: u32,
    readers: [u32; 9],
}

// Coordinate lookups are confined to arrivals and seeds. Slots and four neighbor
// links keep per-cell propagation indexed without filling gaps between distant
// edit neighborhoods; removed slots retain no voxel layers and can be reused.
pub(crate) struct SnapshotAccess<T: VoxelCell> {
    content: ContentCatalog,
    chunks: BTreeMap<(i32, i32), u32>,
    slots: Vec<LightingSlot<T>>,
    free: Vec<u32>,
    min_y: i32,
    height: i32,
    pub(crate) lighting_config: LightingMode,
    track_readers: bool,
}

impl<T: VoxelCell> SnapshotAccess<T> {
    fn cell(&self, pos: IVec3) -> Option<LightCell> {
        let y = pos.y - self.min_y;
        if !(0..self.height).contains(&y) {
            return None;
        }
        Some(LightCell {
            chunk: *self.chunks.get(&(pos.x >> 4, pos.z >> 4))?,
            index: y as u32 * 256 + (pos.z & 15) as u32 * 16 + (pos.x & 15) as u32,
        })
    }

    pub(super) fn neighbor(&self, cell: LightCell, side: usize) -> Option<LightCell> {
        let mut next = cell;
        match side {
            0 if cell.index >= 256 => next.index -= 256,
            1 if cell.index / 256 + 1 < self.height as u32 => next.index += 256,
            0 | 1 => return None,
            2 if cell.index & 240 != 0 => next.index -= 16,
            3 if cell.index & 240 != 240 => next.index += 16,
            4 if cell.index & 15 != 0 => next.index -= 1,
            5 if cell.index & 15 != 15 => next.index += 1,
            _ => {
                next.chunk = self.slots[cell.chunk as usize].neighbors[side - 2]?;
                next.index = match side {
                    2 => cell.index + 240,
                    3 => cell.index - 240,
                    4 => cell.index + 15,
                    _ => cell.index - 15,
                };
            }
        }
        Some(next)
    }

    pub(super) fn definition(&self, cell: LightCell) -> &StateDefinition {
        let chunk = self.slots[cell.chunk as usize].chunk.as_ref().unwrap();
        self.content
            .state(StateId(chunk.states.get(cell.index as usize).id()))
    }

    pub(super) fn light(&self, cell: LightCell) -> u8 {
        self.slots[cell.chunk as usize]
            .chunk
            .as_ref()
            .unwrap()
            .light
            .get(cell.index as usize)
    }

    pub(super) fn set_light(&mut self, cell: LightCell, value: u8, mask: u8) {
        let slot = &mut self.slots[cell.chunk as usize];
        let light = &mut slot.chunk.as_mut().unwrap().light;
        let value = (light.get(cell.index as usize) & !mask) | (value & mask);
        if light.set(cell.index as usize, value) {
            slot.changed |= 1 << (cell.index as usize / SECTION_VOLUME);
            if self.track_readers {
                let local = hollow_core::coordinates::cell_position(cell.index as usize);
                let sections = crate::chunk::ChunkVoxels::<T>::sections_around(local.y);
                slot.readers[4] |= sections;
                for offset in crate::edits::border_neighbors(local) {
                    slot.readers[((offset.y + 1) * 3 + offset.x + 1) as usize] |= sections;
                }
            }
        }
    }

    fn chunk(&self, position: &(i32, i32)) -> Option<&LightingChunk<T>> {
        self.slots[*self.chunks.get(position)? as usize]
            .chunk
            .as_ref()
    }

    fn insert(&mut self, position: (i32, i32), snapshot: LightingChunk<T>) {
        let id = self.free.pop().unwrap_or_else(|| {
            let id = u32::try_from(self.slots.len()).expect("Lighting slot index fits u32");
            self.slots.push(LightingSlot {
                chunk: None,
                neighbors: [None; 4],
                changed: 0,
                readers: [0; 9],
            });
            id
        });
        self.slots[id as usize].chunk = Some(snapshot);
        self.chunks.insert(position, id);
        for (side, (dx, dz)) in [(0, -1), (0, 1), (-1, 0), (1, 0)].into_iter().enumerate() {
            let neighbor = self
                .chunks
                .get(&(position.0 + dx, position.1 + dz))
                .copied();
            self.slots[id as usize].neighbors[side] = neighbor;
            if let Some(neighbor) = neighbor {
                self.slots[neighbor as usize].neighbors[side ^ 1] = Some(id);
            }
        }
    }

    fn remove(&mut self, position: &(i32, i32)) {
        let Some(id) = self.chunks.remove(position) else {
            return;
        };
        for (side, neighbor) in self.slots[id as usize].neighbors.into_iter().enumerate() {
            if let Some(neighbor) = neighbor {
                self.slots[neighbor as usize].neighbors[side ^ 1] = None;
            }
        }
        self.slots[id as usize].chunk = None;
        self.slots[id as usize].changed = 0;
        self.free.push(id);
    }
}

impl<T: VoxelCell> LightingSnapshots<T> {
    pub fn census(&self, census: &mut hollow_core::sections::AllocationCensus) {
        census.bytes += self.access.slots.capacity() * size_of::<LightingSlot<T>>()
            + self.access.free.capacity() * size_of::<u32>();
        for slot in &self.access.slots {
            if let Some(chunk) = &slot.chunk {
                chunk.states.census(census);
                chunk.light.census(census);
                census.bytes += chunk
                    .changed_cells
                    .as_ref()
                    .map_or(0, |cells| cells.capacity() * size_of::<u32>());
            }
        }
    }

    pub fn new(min_y: i32, height: i32, content: ContentCatalog) -> Self {
        assert!(
            height > 0 && height as u64 * 256 <= u64::from(u32::MAX),
            "Lighting column index fits u32"
        );
        Self {
            access: SnapshotAccess {
                content,
                chunks: BTreeMap::new(),
                slots: Vec::new(),
                free: Vec::new(),
                min_y,
                height,
                lighting_config: LightingMode::Default,
                track_readers: false,
            },
        }
    }
    pub(crate) fn prepare(&mut self, min_y: i32, height: i32, content: ContentCatalog) {
        self.access.min_y = min_y;
        self.access.height = height;
        self.access.content = content;
    }
    pub(crate) fn block_edits(
        &mut self,
        positions: impl Iterator<Item = IVec3>,
        engine: &mut SnapshotLightEngine,
    ) {
        self.access.track_readers = true;
        for position in positions {
            if let Some(cell) = self.access.cell(position) {
                engine.invalidate_block(&mut self.access, cell);
            }
        }
        engine.perform_block_light_updates(&mut self.access);
    }
    pub(crate) fn insert(&mut self, snapshot: LightingChunk<T>) {
        self.access.insert((snapshot.x, snapshot.z), snapshot);
    }
    pub(crate) fn release_excess(&mut self) {
        self.access.slots.shrink_to(64);
    }
    pub(crate) fn drain(&mut self) -> impl Iterator<Item = (i32, i32, Layer<u8>, [u32; 9])> + '_ {
        self.access.chunks.clear();
        self.access.free.clear();
        self.access
            .slots
            .drain(..)
            .filter(|slot| slot.changed != 0)
            .map(|slot| {
                let chunk = slot.chunk.unwrap();
                (chunk.x, chunk.z, chunk.light, slot.readers)
            })
    }
    pub fn update(
        &mut self,
        snapshots: Vec<LightingChunk<T>>,
        removed: &[(i32, i32)],
    ) -> Vec<LightingUpdate> {
        let access = &mut self.access;
        let (min_y, height) = (access.min_y, access.height);
        for position in removed {
            access.remove(position);
        }
        let mut rebuild = BTreeSet::new();
        let mut block_changes = Vec::new();
        let mut sky_columns = BTreeSet::new();
        let mut borders = BTreeSet::new();
        for mut snapshot in snapshots {
            let position = (snapshot.x, snapshot.z);
            if let Some(&slot) = access.chunks.get(&position) {
                let old = access.slots[slot as usize].chunk.as_mut().unwrap();
                let mut changed_cell = |i: u32| {
                    let previous = old.states.get(i as usize);
                    let next = snapshot.states.get(i as usize);
                    if previous == next {
                        return;
                    }
                    let previous = access.content.state(StateId(previous.id()));
                    let next = access.content.state(StateId(next.id()));
                    if previous.opacity != next.opacity || previous.luminance != next.luminance {
                        block_changes.push(LightCell {
                            chunk: slot,
                            index: i,
                        });
                    }
                    if previous.opacity != next.opacity
                        || previous.can_occlude() != next.can_occlude()
                        || previous.is_solid_render() != next.is_solid_render()
                    {
                        sky_columns.insert((slot, i & 255));
                    }
                };
                if let Some(cells) = snapshot.changed_cells.take() {
                    for i in cells {
                        changed_cell(i);
                    }
                } else {
                    for section in 0..snapshot.states.section_count() {
                        let previous = old.states.section(section);
                        let next = snapshot.states.section(section);
                        if !previous.shared(next) {
                            for i in section * SECTION_VOLUME..(section + 1) * SECTION_VOLUME {
                                changed_cell(i as u32);
                            }
                        }
                    }
                }
                old.states = snapshot.states;
                old.modified = snapshot.modified;
            } else {
                if snapshot.modified {
                    rebuild.insert(position);
                }
                access.insert(position, snapshot);
                borders.insert(position);
            }
        }
        for &(x, z) in &borders {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let p = (x + dx, z + dz);
                    if access.chunk(&p).is_some_and(|chunk| chunk.modified) {
                        rebuild.insert(p);
                    }
                }
            }
        }
        let mut reset = BTreeSet::new();
        for (x, z) in rebuild {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let p = (x + dx, z + dz);
                    if access.chunks.contains_key(&p) {
                        reset.insert(p);
                    }
                }
            }
        }
        for p in &reset {
            let slot = &mut access.slots[access.chunks[p] as usize];
            let chunk = slot.chunk.as_mut().unwrap();
            chunk.light = Layer::filled(0, chunk.light.section_count());
            slot.changed = hollow_core::coordinates::section_mask(chunk.light.section_count());
        }
        let mut engine = SnapshotLightEngine::default();
        for &(x, z) in &reset {
            let slot = access.chunks[&(x, z)];
            let states = access.chunk(&(x, z)).unwrap().states.clone();
            for section in 0..states.section_count() {
                if !states
                    .section(section)
                    .any(|state| access.content.state(StateId(state.id())).luminance > 0)
                {
                    continue;
                }
                for i in section * SECTION_VOLUME..(section + 1) * SECTION_VOLUME {
                    let emission = access.content.state(StateId(states.get(i).id())).luminance;
                    if emission == 0 {
                        continue;
                    }
                    let pos = LightCell {
                        chunk: slot,
                        index: i as u32,
                    };
                    access.set_light(pos, emission, 15);
                    engine.queue_block_light_increase(pos, emission);
                }
            }
            for dz in 0..16 {
                for dx in 0..16 {
                    engine.seed_sky(
                        access,
                        LightCell {
                            chunk: slot,
                            index: (height as u32 - 1) * 256 + dz * 16 + dx,
                        },
                    );
                }
            }
        }
        let edges: BTreeSet<_> = reset
            .iter()
            .copied()
            .chain(borders.iter().copied())
            .collect();
        for (x, z) in edges {
            for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                if !access.chunks.contains_key(&(x + dx, z + dz)) {
                    continue;
                }
                for y in min_y..min_y + height {
                    for offset in 0..16 {
                        let (px, pz) = match (dx, dz) {
                            (-1, _) => (x * 16, z * 16 + offset),
                            (1, _) => (x * 16 + 15, z * 16 + offset),
                            (_, -1) => (x * 16 + offset, z * 16),
                            _ => (x * 16 + offset, z * 16 + 15),
                        };
                        for (bx, bz) in [(px, pz), (px + dx, pz + dz)] {
                            let pos = access.cell(IVec3::new(bx, y, bz)).unwrap();
                            let value = access.light(pos);
                            if value & 15 > 0 {
                                engine.queue_block_light_increase(pos, value & 15);
                            }
                            if value >> 4 > 0 {
                                engine.queue_sky_light_increase(pos, value >> 4);
                            }
                        }
                    }
                }
            }
        }
        for pos in block_changes {
            engine.invalidate_block(access, pos);
        }
        for (chunk, xz) in sky_columns {
            engine.invalidate_sky_column(access, chunk, xz, height as u32);
        }
        engine.perform_block_light_updates(access);
        engine.perform_sky_light_updates(access);
        let mut updates = Vec::new();
        for (&(x, z), &id) in &access.chunks {
            let slot = &mut access.slots[id as usize];
            let changed = std::mem::take(&mut slot.changed);
            if changed != 0 {
                slot.chunk.as_mut().unwrap().light.compact_sections(changed);
                updates.push(LightingUpdate {
                    x,
                    z,
                    light: slot.chunk.as_ref().unwrap().light.clone(),
                });
            }
        }
        updates
    }
}
