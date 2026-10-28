//! Edit lighting over shared sections, using the generation engine's propagation rules.
use super::propagation::{LightingMode, SnapshotLightEngine};
use bevy_math::IVec3;
use hollow_core::{
    StateId, VoxelCell,
    content::{ContentCatalog, StateDefinition},
    coordinates::SECTION_VOLUME,
    sections::{Layer, Section},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct LightingChunk<T: VoxelCell> {
    pub x: i32,
    pub z: i32,
    pub states: Layer<T>,
    pub light: Layer<u8>,
    pub modified: bool,
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
    chunk: u32,
    index: u32,
}

struct LightingSlot<T: VoxelCell> {
    chunk: Option<LightingChunk<T>>,
    neighbors: [Option<u32>; 4],
    changed: bool,
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
        slot.changed |= light.set(cell.index as usize, value);
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
                changed: false,
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
        self.slots[id as usize].changed = false;
        self.free.push(id);
    }
}

impl<T: VoxelCell> LightingSnapshots<T> {
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
            },
        }
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
        let mut rebuild = BTreeMap::new();
        let mut borders = BTreeSet::new();
        for snapshot in snapshots {
            let position = (snapshot.x, snapshot.z);
            if let Some(&slot) = access.chunks.get(&position) {
                let old = access.slots[slot as usize].chunk.as_mut().unwrap();
                let changed = old.states.changed_sections(&snapshot.states);
                for section in 0..snapshot.states.section_count() {
                    if changed & (1 << section) == 0 {
                        continue;
                    }
                    for i in section * SECTION_VOLUME..(section + 1) * SECTION_VOLUME {
                        let old = access.content.state(StateId(old.states.get(i).id()));
                        let new = access.content.state(StateId(snapshot.states.get(i).id()));
                        let sky_changed = old.opacity != new.opacity
                            || old.can_occlude() != new.can_occlude()
                            || old.is_solid_render() != new.is_solid_render();
                        if sky_changed || old.luminance != new.luminance {
                            *rebuild.entry(position).or_insert(false) |= sky_changed;
                        }
                    }
                }
                old.states = snapshot.states;
                old.modified = snapshot.modified;
            } else {
                if snapshot.modified {
                    rebuild.insert(position, true);
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
                        rebuild.insert(p, true);
                    }
                }
            }
        }
        let mut reset = BTreeMap::new();
        for ((x, z), sky) in rebuild {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let p = (x + dx, z + dz);
                    if access.chunks.contains_key(&p) {
                        *reset.entry(p).or_insert(false) |= sky;
                    }
                }
            }
        }
        for (p, &sky) in &reset {
            let slot = &mut access.slots[access.chunks[p] as usize];
            let chunk = slot.chunk.as_mut().unwrap();
            chunk.light = Layer::from_sections((0..chunk.light.section_count()).map(|section| {
                if sky {
                    return Section::Uniform(0);
                }
                match chunk.light.section(section) {
                    Section::Uniform(value) => Section::Uniform(value & 240),
                    section => {
                        Section::from_cells(&std::array::from_fn::<_, SECTION_VOLUME, _>(|i| {
                            section.get(i) & 240
                        }))
                    }
                }
            }));
            slot.changed = true;
        }
        let mut engine = SnapshotLightEngine::default();
        for (&(x, z), &sky) in &reset {
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
            if sky {
                for dz in 0..16 {
                    for dx in 0..16 {
                        engine.check_sky_light_updates(
                            access,
                            LightCell {
                                chunk: slot,
                                index: (height as u32 - 1) * 256 + dz * 16 + dx,
                            },
                        );
                    }
                }
            }
        }
        let edges: BTreeSet<_> = reset
            .keys()
            .copied()
            .chain(borders.iter().copied())
            .collect();
        for (x, z) in edges {
            let sky = reset.get(&(x, z)).copied().unwrap_or(false) || borders.contains(&(x, z));
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
                            if sky && value >> 4 > 0 {
                                engine.queue_sky_light_increase(pos, value >> 4);
                            }
                        }
                    }
                }
            }
        }
        engine.perform_block_light_updates(access);
        engine.perform_sky_light_updates(access);
        let mut updates = Vec::new();
        for (&(x, z), &id) in &access.chunks {
            let slot = &mut access.slots[id as usize];
            if std::mem::take(&mut slot.changed) {
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
