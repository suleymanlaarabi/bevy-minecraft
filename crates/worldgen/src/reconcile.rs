//! Edit lighting over shared sections, using the generation engine's propagation rules.
use crate::lighting::LightingEngineConfig;
use crate::lighting::snapshots::SnapshotLightEngine;
use crate::{
    VoxelState,
    coordinates::SECTION_VOLUME,
    sections::{Layer, Section},
};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub struct LightingChunk {
    pub x: i32,
    pub z: i32,
    pub states: Layer<VoxelState>,
    pub light: Layer<u8>,
    pub modified: bool,
}
pub struct LightingUpdate {
    pub x: i32,
    pub z: i32,
    pub light: Layer<u8>,
}

/// Previous inputs retain the same shared allocations as the game. No world,
/// runtime, server chunks, palette conversion, or dense column mirror is created.
pub struct LightingSnapshots {
    access: SnapshotAccess,
}
pub(crate) struct SnapshotAccess {
    chunks: BTreeMap<(i32, i32), LightingChunk>,
    changed: BTreeSet<(i32, i32)>,
    min_y: i32,
    height: i32,
    pub(crate) lighting_config: LightingEngineConfig,
}
impl SnapshotAccess {
    fn index(&self, pos: &BlockPos) -> Option<((i32, i32), usize)> {
        let y = pos.0.y - self.min_y;
        (0..self.height).contains(&y).then_some((
            (pos.0.x >> 4, pos.0.z >> 4),
            y as usize * 256 + (pos.0.z & 15) as usize * 16 + (pos.0.x & 15) as usize,
        ))
    }
    pub(crate) fn is_chunk_loaded(&self, pos: &Vector2<i32>) -> bool {
        self.chunks.contains_key(&(pos.x, pos.y))
    }
    pub(crate) fn get_block_state(&self, pos: &BlockPos) -> VoxelState {
        self.index(pos)
            .and_then(|(p, i)| self.chunks.get(&p).map(|chunk| chunk.states.get(i)))
            .unwrap_or(VoxelState::AIR)
    }
    pub(crate) fn light(&self, pos: &BlockPos) -> Option<u8> {
        let (p, i) = self.index(pos)?;
        Some(self.chunks.get(&p)?.light.get(i))
    }
    pub(crate) fn set_light(&mut self, pos: &BlockPos, value: u8, mask: u8) -> Result<(), String> {
        let (p, i) = self.index(pos).ok_or("Invalid light position")?;
        let chunk = self.chunks.get_mut(&p).ok_or("Unloaded light position")?;
        let value = (chunk.light.get(i) & !mask) | (value & mask);
        if chunk.light.set(i, value) {
            self.changed.insert(p);
        }
        Ok(())
    }
}

impl LightingSnapshots {
    pub fn new(min_y: i32, height: i32) -> Self {
        Self {
            access: SnapshotAccess {
                chunks: BTreeMap::new(),
                changed: BTreeSet::new(),
                min_y,
                height,
                lighting_config: LightingEngineConfig::Default,
            },
        }
    }
    pub fn update(
        &mut self,
        snapshots: Vec<LightingChunk>,
        removed: &[(i32, i32)],
    ) -> Vec<LightingUpdate> {
        let access = &mut self.access;
        let (min_y, height) = (access.min_y, access.height);
        for position in removed {
            access.chunks.remove(position);
        }
        let mut rebuild = BTreeMap::new();
        let mut borders = BTreeSet::new();
        for snapshot in snapshots {
            let position = (snapshot.x, snapshot.z);
            if let Some(old) = access.chunks.get_mut(&position) {
                let changed = old.states.changed_sections(&snapshot.states);
                for section in 0..snapshot.states.section_count() {
                    if changed & (1 << section) == 0 {
                        continue;
                    }
                    for i in section * SECTION_VOLUME..(section + 1) * SECTION_VOLUME {
                        let old = old.states.get(i).definition();
                        let new = snapshot.states.get(i).definition();
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
                access.chunks.insert(position, snapshot);
                borders.insert(position);
            }
        }
        for &(x, z) in &borders {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let p = (x + dx, z + dz);
                    if access.chunks.get(&p).is_some_and(|chunk| chunk.modified) {
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
            let chunk = access.chunks.get_mut(p).unwrap();
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
            access.changed.insert(*p);
        }
        let mut engine = SnapshotLightEngine::new(min_y, min_y + height);
        for (&(x, z), &sky) in &reset {
            let states = access.chunks[&(x, z)].states.clone();
            for section in 0..states.section_count() {
                if !states
                    .section(section)
                    .any(|state| state.definition().luminance > 0)
                {
                    continue;
                }
                for i in section * SECTION_VOLUME..(section + 1) * SECTION_VOLUME {
                    let emission = states.get(i).definition().luminance;
                    if emission == 0 {
                        continue;
                    }
                    let pos = BlockPos::new(
                        x * 16 + (i & 15) as i32,
                        min_y + (i >> 8) as i32,
                        z * 16 + ((i >> 4) & 15) as i32,
                    );
                    engine
                        .set_block_light_level(access, &pos, emission)
                        .unwrap();
                    engine.queue_block_light_increase(pos, emission);
                }
            }
            if sky {
                for dz in 0..16 {
                    for dx in 0..16 {
                        engine.check_sky_light_updates(
                            access,
                            BlockPos::new(x * 16 + dx, min_y + height - 1, z * 16 + dz),
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
                            let pos = BlockPos::new(bx, y, bz);
                            if let Some(value) = engine.get_block_light_level(access, &pos)
                                && value > 0
                            {
                                engine.queue_block_light_increase(pos, value);
                            }
                            if sky {
                                let value = engine.get_sky_light_level(access, &pos);
                                if value > 0 {
                                    engine.queue_sky_light_increase(pos, value);
                                }
                            }
                        }
                    }
                }
            }
        }
        engine.perform_block_light_updates(access);
        engine.perform_sky_light_updates(access);
        std::mem::take(&mut access.changed)
            .into_iter()
            .filter_map(|(x, z)| {
                access.chunks.get(&(x, z)).map(|chunk| LightingUpdate {
                    x,
                    z,
                    light: chunk.light.clone(),
                })
            })
            .collect()
    }
}
