//! Compact column work. Sparse sections allocate their deduplication bitsets on demand.
use super::*;
use crate::voxel::{
    VoxelState,
    edits::{CheckedEdit, VoxelEditWorld},
};
use bevy::{ecs::system::SystemParam, platform::collections::HashMap};
use hollow_core::coordinates::{CHUNK_HEIGHT, cell_index};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const NEIGHBOR_CONNECTION: u8 = 1;
pub(crate) const NEIGHBOR_SUPPORT: u8 = 2;
pub(crate) const NEIGHBOR_FLUID: u8 = 4;
pub(crate) const NEIGHBOR_ALL: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum TickChannel {
    Block,
    Fluid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ScheduledTick {
    pub due: u64,
    pub priority: i8,
    pub order: u64,
    pub cell: u32,
    pub channel: TickChannel,
    pub expected: u32,
}
#[derive(Default, Clone)]
struct ScheduledWork {
    ordered: BTreeSet<ScheduledTick>,
    // Keys are sparse cell/channel pairs. An index permits direct cancellation,
    // unlike a heap with stale entries retained until their original deadlines.
    cells: BTreeMap<(u32, TickChannel), ScheduledTick>,
}
impl ScheduledWork {
    fn insert(&mut self, tick: ScheduledTick) {
        let key = (tick.cell, tick.channel);
        if let Some(previous) = self.cells.get(&key) {
            if previous.expected == tick.expected {
                return;
            }
            self.ordered.remove(previous);
        }
        self.cells.insert(key, tick);
        self.ordered.insert(tick);
    }
    fn pop(&mut self, now: u64) -> Option<ScheduledTick> {
        if self.ordered.first()?.due > now {
            return None;
        }
        let tick = self.ordered.pop_first()?;
        self.cells.remove(&(tick.cell, tick.channel));
        Some(tick)
    }
}
#[derive(Clone)]
struct NeighborBits {
    planes: [[u64; 64]; 3],
    words: u64,
}
impl NeighborBits {
    fn insert(&mut self, cell: u16, reasons: u8) -> bool {
        let (word, bit) = (usize::from(cell) / 64, usize::from(cell) % 64);
        let occupied = self
            .planes
            .iter()
            .any(|plane| plane[word] & (1 << bit) != 0);
        self.words |= 1 << word;
        for (i, plane) in self.planes.iter_mut().enumerate() {
            if reasons & (1 << i) != 0 {
                plane[word] |= 1 << bit;
            }
        }
        !occupied
    }
    fn pop(&mut self) -> Option<(u16, u8)> {
        if self.words == 0 {
            return None;
        }
        let word = self.words.trailing_zeros() as usize;
        let bit = self
            .planes
            .iter()
            .fold(0, |bits, plane| bits | plane[word])
            .trailing_zeros();
        let mut reasons = 0;
        for (i, plane) in self.planes.iter_mut().enumerate() {
            if plane[word] & (1 << bit) != 0 {
                reasons |= 1 << i;
                plane[word] &= !(1 << bit);
            }
        }
        if self.planes.iter().all(|plane| plane[word] == 0) {
            self.words &= !(1 << word);
        }
        Some(((word * 64) as u16 + bit as u16, reasons))
    }
}
#[derive(Clone)]
enum NeighborSection {
    Sparse(Vec<(u16, u8)>),
    Dense(Box<NeighborBits>),
}
impl Default for NeighborSection {
    fn default() -> Self {
        Self::Sparse(Vec::new())
    }
}
impl NeighborSection {
    fn insert(&mut self, cell: u16, reasons: u8) -> bool {
        match self {
            Self::Dense(bits) => bits.insert(cell, reasons),
            Self::Sparse(cells) => {
                if let Some((_, pending)) = cells.iter_mut().find(|(pending, _)| *pending == cell) {
                    *pending |= reasons;
                    return false;
                }
                cells.push((cell, reasons));
                // Isolated edits retain only a small vector. At 64 cells, trade
                // 1.5 KiB for constant-cost merging throughout dense cascades.
                if cells.len() == 64 {
                    let mut bits = Box::new(NeighborBits {
                        planes: [[0; 64]; 3],
                        words: 0,
                    });
                    for &(cell, reasons) in cells.iter() {
                        bits.insert(cell, reasons);
                    }
                    *self = Self::Dense(bits);
                }
                true
            }
        }
    }
    fn pop(&mut self) -> Option<(u16, u8)> {
        match self {
            Self::Sparse(cells) => (!cells.is_empty()).then(|| cells.remove(0)),
            Self::Dense(bits) => bits.pop(),
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::Sparse(cells) => cells.is_empty(),
            Self::Dense(bits) => bits.words == 0,
        }
    }
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Sparse(cells) => cells.capacity() * std::mem::size_of::<(u16, u8)>(),
            Self::Dense(_) => std::mem::size_of::<NeighborBits>(),
        }
    }
    fn capture(&self, section: u8, output: &mut Vec<(u32, u8)>) {
        match self {
            Self::Sparse(cells) => output.extend(
                cells
                    .iter()
                    .map(|&(cell, reasons)| (u32::from(section) * 4096 + u32::from(cell), reasons)),
            ),
            Self::Dense(bits) => {
                let mut words = bits.words;
                while words != 0 {
                    let word = words.trailing_zeros() as usize;
                    words &= words - 1;
                    let mut merged = bits
                        .planes
                        .iter()
                        .fold(0, |value, plane| value | plane[word]);
                    while merged != 0 {
                        let bit = merged.trailing_zeros();
                        merged &= merged - 1;
                        let reasons =
                            bits.planes
                                .iter()
                                .enumerate()
                                .fold(0, |reasons, (i, plane)| {
                                    reasons | (u8::from(plane[word] & (1 << bit) != 0) << i)
                                });
                        output.push((u32::from(section) * 4096 + word as u32 * 64 + bit, reasons));
                    }
                }
            }
        }
    }
}
#[derive(Component, Default, Clone)]
pub(crate) struct ColumnSimulation {
    pub local_tick: u64,
    pub dormant_drops: Vec<crate::world::SavedDrop>,
    order: u64,
    scheduled: Option<Box<ScheduledWork>>,
    // Usually only one or two of the 24 sections have work. A sparse tree avoids
    // reserving an array of vector-bearing entries on every otherwise idle owner.
    neighbors: BTreeMap<u8, NeighborSection>,
    // Frontier keys are sparse. A direct cell/channel table would reserve several
    // MiB per owner even when no destination is blocked.
    pub suspended: BTreeMap<(u32, PendingChannel), BoundaryWork>,
    neighbor_count: usize,
    pub random_sections: u32,
    random_dirty: u32,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PendingChannel {
    Neighbor,
    Block,
    Fluid,
}
impl From<TickChannel> for PendingChannel {
    fn from(channel: TickChannel) -> Self {
        match channel {
            TickChannel::Block => Self::Block,
            TickChannel::Fluid => Self::Fluid,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct BoundaryWork {
    pub chunk: [i32; 2],
    cell: u32,
    reasons: u8,
    tick: Option<ScheduledTick>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct SimulationSnapshot {
    local_tick: u64,
    order: u64,
    scheduled: Vec<ScheduledTick>,
    neighbors: Vec<(u32, u8)>,
    suspended: Vec<BoundaryWork>,
}
impl ColumnSimulation {
    pub(crate) fn wake(&mut self, chunk: IVec2, ready: &mut Vec<BoundaryWork>) {
        ready.extend(
            self.suspended
                .extract_if(.., |_, task| task.chunk == chunk.to_array())
                .map(|(_, task)| task),
        );
        for task in ready.drain(..) {
            if let Some(tick) = task.tick {
                self.schedule(tick.cell, tick.channel, tick.expected, 1, tick.priority);
            } else {
                self.enqueue(task.cell, task.reasons);
            }
        }
    }

    fn invalidate(&mut self, cell: u32, block: u32, fluid: u32) {
        for (channel, expected) in [(TickChannel::Block, block), (TickChannel::Fluid, fluid)] {
            if let Some(work) = self.scheduled.as_mut() {
                if let Some(tick) = work.cells.get(&(cell, channel)).copied()
                    && tick.expected != expected
                {
                    work.cells.remove(&(cell, channel));
                    work.ordered.remove(&tick);
                }
            }
            let key = (cell, channel.into());
            if self
                .suspended
                .get(&key)
                .and_then(|task| task.tick)
                .is_some_and(|tick| tick.expected != expected)
            {
                self.suspended.remove(&key);
            }
        }
        if self
            .scheduled
            .as_ref()
            .is_some_and(|work| work.ordered.is_empty())
        {
            self.scheduled = None;
        }
    }
    pub(crate) fn defer_tick(&mut self, tick: ScheduledTick) {
        self.scheduled.get_or_insert_default().insert(tick);
    }
    pub(crate) fn invalidate_random_sections(&mut self, sections: u32) {
        self.random_dirty |= sections;
    }
    pub(crate) fn snapshot_bytes(&self) -> usize {
        self.scheduled.as_ref().map_or(0, |work| work.cells.len())
            * std::mem::size_of::<ScheduledTick>()
            + self.neighbor_count * std::mem::size_of::<(u32, u8)>()
            + self.suspended.len() * std::mem::size_of::<BoundaryWork>()
    }
    pub(crate) fn capture_reservation(&self) -> usize {
        // Include compact copies, JSON vector growth, compression input/output
        // buffers and allocator headroom, rather than counting tree handles alone.
        4096 + self.scheduled.as_ref().map_or(0, |work| work.cells.len()) * 1024
            + self.neighbor_count * 128
            + self.suspended.len() * 2048
    }
    pub(crate) fn estimated_bytes(&self) -> usize {
        // Charge shared allocations in full: capture/encoding also retain them.
        self.neighbors
            .values()
            .map(|section| std::mem::size_of::<NeighborSection>() + section.heap_bytes() + 512)
            .sum::<usize>()
            + self.scheduled.as_ref().map_or(0, |work| {
                work.cells.len() * (2 * std::mem::size_of::<ScheduledTick>() + 128)
            })
            + self.suspended.len() * (std::mem::size_of::<BoundaryWork>() + 128)
            + self.dormant_drops.capacity() * std::mem::size_of::<crate::world::SavedDrop>()
    }
    pub(crate) fn ready_count(&self) -> usize {
        self.neighbor_count + usize::from(self.has_due())
    }
    pub(crate) fn has_neighbors(&self) -> bool {
        !self.neighbors.is_empty()
    }
    pub(crate) fn has_due(&self) -> bool {
        self.scheduled
            .as_ref()
            .and_then(|work| work.ordered.first())
            .is_some_and(|tick| tick.due <= self.local_tick)
    }
    pub(crate) fn pop_tick(&mut self) -> Option<ScheduledTick> {
        let tick = self.scheduled.as_mut()?.pop(self.local_tick);
        if self
            .scheduled
            .as_ref()
            .is_some_and(|work| work.ordered.is_empty())
        {
            self.scheduled = None;
        }
        tick
    }
    pub(crate) fn suspend_tick(&mut self, chunk: IVec2, tick: ScheduledTick) {
        self.suspended.insert(
            (tick.cell, tick.channel.into()),
            BoundaryWork {
                chunk: chunk.to_array(),
                cell: tick.cell,
                reasons: 0,
                tick: Some(tick),
            },
        );
    }
    pub(crate) fn enqueue(&mut self, cell: u32, reasons: u8) {
        if reasons == 0 {
            return;
        }
        if let Some(task) = self.suspended.get_mut(&(cell, PendingChannel::Neighbor)) {
            task.reasons |= reasons;
            return;
        }
        let section = (cell / 4096) as u8;
        self.neighbor_count += usize::from(
            self.neighbors
                .entry(section)
                .or_default()
                .insert((cell % 4096) as u16, reasons),
        );
    }
    pub(crate) fn schedule(
        &mut self,
        cell: u32,
        channel: TickChannel,
        expected: u32,
        delay: u64,
        priority: i8,
    ) {
        let suspended_key = (cell, channel.into());
        if let Some(task) = self.suspended.get(&suspended_key) {
            if task.tick.is_some_and(|tick| tick.expected == expected) {
                return;
            }
            self.suspended.remove(&suspended_key);
        }
        if self
            .scheduled
            .as_ref()
            .and_then(|work| work.cells.get(&(cell, channel)))
            .is_some_and(|tick| tick.expected == expected)
        {
            return;
        }
        let tick = ScheduledTick {
            due: self.local_tick.saturating_add(delay.max(1)),
            priority,
            order: self.order,
            cell,
            channel,
            expected,
        };
        self.order = self
            .order
            .checked_add(1)
            .expect("Scheduled insertion counter exhausted");
        self.scheduled.get_or_insert_default().insert(tick);
    }
    fn pop_neighbor(&mut self) -> Option<(u32, u8)> {
        let mut section = self.neighbors.first_entry()?;
        let (cell, reasons) = section.get_mut().pop()?;
        let index = u32::from(*section.key()) * 4096 + u32::from(cell);
        if section.get().is_empty() {
            section.remove();
        }
        self.neighbor_count -= 1;
        Some((index, reasons))
    }
    pub fn capture(&self) -> SimulationSnapshot {
        let mut neighbors = Vec::with_capacity(self.neighbor_count);
        for (&section, work) in &self.neighbors {
            work.capture(section, &mut neighbors);
        }
        SimulationSnapshot {
            local_tick: self.local_tick,
            order: self.order,
            scheduled: self
                .scheduled
                .as_ref()
                .map_or_else(Vec::new, |work| work.ordered.iter().copied().collect()),
            neighbors,
            suspended: self.suspended.values().cloned().collect(),
        }
    }
    pub fn restore(snapshot: SimulationSnapshot) -> Result<Self, std::io::Error> {
        let mut work = Self {
            local_tick: snapshot.local_tick,
            order: snapshot.order,
            ..default()
        };
        let valid = |cell| cell < CHUNK_HEIGHT as u32 * 256;
        if snapshot.scheduled.iter().any(|tick| !valid(tick.cell))
            || snapshot.neighbors.iter().any(|&(cell, reasons)| {
                !valid(cell) || reasons == 0 || reasons & !NEIGHBOR_ALL != 0
            })
            || snapshot.suspended.iter().any(|task| !valid(task.cell))
        {
            return Err(std::io::Error::other("Invalid column simulation work"));
        }
        for tick in snapshot.scheduled {
            work.scheduled.get_or_insert_default().insert(tick);
        }
        for (cell, reasons) in snapshot.neighbors {
            work.enqueue(cell, reasons);
        }
        for task in snapshot.suspended {
            let channel = task
                .tick
                .map_or(PendingChannel::Neighbor, |tick| tick.channel.into());
            work.suspended.insert((task.cell, channel), task);
        }
        Ok(work)
    }
}
// Chunk coordinates are genuinely sparse and unbounded. Each arrival touches only
// the owners indexed by that frontier, never all suspended work in the world.
#[derive(Resource, Default)]
pub(crate) struct BoundaryWaits(HashMap<IVec2, Vec<Entity>>);
impl BoundaryWaits {
    pub(crate) fn remove_owner(&mut self, owner: Entity) {
        self.0.retain(|_, owners| {
            owners.retain(|entity| *entity != owner);
            !owners.is_empty()
        });
    }
    pub(crate) fn wake(&mut self, chunk: IVec2, ready: &mut Vec<(IVec2, Entity)>) {
        ready.extend(
            self.0
                .remove(&chunk)
                .into_iter()
                .flatten()
                .map(|owner| (chunk, owner)),
        );
    }
    pub(crate) fn register(&mut self, chunk: IVec2, owner: Entity) {
        let owners = self.0.entry(chunk).or_default();
        if !owners.contains(&owner) {
            owners.push(owner);
        }
    }
}
/// Neighbor scheduling borrows only the terrain and per-column work it touches.
#[derive(SystemParam)]
pub(crate) struct NeighborUpdates<'w, 's> {
    terrain: crate::voxel::VoxelWorld<'w, 's>,
    owners: Res<'w, GameplayColumns>,
    work: Query<'w, 's, &'static mut ColumnSimulation>,
    fluids: Res<'w, super::fluids::FluidCatalog>,
}
impl NeighborUpdates<'_, '_> {
    pub(crate) fn enqueue(&mut self, position: IVec3) {
        let center = crate::voxel::split_world_position(position).0;
        let center_owner = self.owners.get(&center).copied();
        for delta in [
            IVec3::ZERO,
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ] {
            let target = position + delta;
            let (chunk, local) = crate::voxel::split_world_position(target);
            let Some(cell) = cell_index(local) else {
                continue;
            };
            let owner = if chunk == center {
                center_owner
            } else {
                self.owners.get(&chunk).copied()
            };
            let Some(mut work) = owner.and_then(|owner| self.work.get_mut(owner).ok()) else {
                continue;
            };
            if delta == IVec3::ZERO {
                if let Some(state) = self.terrain.state(target) {
                    work.invalidate(
                        cell as u32,
                        state.kind().0,
                        u32::from(self.fluids.info(state).family.0),
                    );
                }
                work.invalidate_random_sections(1 << (cell / 4096));
            }
            work.enqueue(cell as u32, NEIGHBOR_ALL);
        }
    }
}
pub(crate) fn cell_position(chunk: IVec2, cell: u32) -> IVec3 {
    IVec3::new(
        chunk.x * 16 + (cell & 15) as i32,
        (cell >> 8) as i32,
        chunk.y * 16 + ((cell >> 4) & 15) as i32,
    )
}

pub(crate) fn process_neighbors(
    mut commands: Commands,
    mut world: VoxelEditWorld,
    mut columns: Query<
        (Entity, &GameplayColumn, &mut ColumnSimulation),
        (With<Simulating>, With<ReadyNeighbors>),
    >,
    mut budget: ResMut<SimulationBudget>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut waits: ResMut<BoundaryWaits>,
    fluids: Res<super::fluids::FluidCatalog>,
    mut cursor: Local<usize>,
    // Eight coordinate bytes per entry avoid ECS lookups inside sort comparisons.
    mut owners: Local<Vec<((i32, i32), Entity)>>,
) {
    owners.clear();
    owners.extend(
        columns
            .iter()
            .map(|(entity, column, _)| ((column.position.x, column.position.y), entity)),
    );
    owners.sort_unstable_by_key(|&(position, _)| position);
    if owners.is_empty() {
        return;
    }
    let mut idle = 0;
    while budget.neighbors > 0 && budget.mutations >= 2 && idle < owners.len() {
        *cursor %= owners.len();
        let (_, owner) = owners[*cursor];
        *cursor += 1;
        let Ok((_, column, mut work)) = columns.get_mut(owner) else {
            continue;
        };
        let Some((cell, reasons)) = work.pop_neighbor() else {
            idle += 1;
            continue;
        };
        idle = 0;
        budget.neighbors -= 1;
        diagnostics.neighbor_cells += 1;
        let position = cell_position(column.position, cell);
        let Some(state) = world.state(position) else {
            continue;
        };
        let mut missing = None;
        let next = if reasons & NEIGHBOR_CONNECTION != 0 {
            crate::voxel::rules::connected(position, state, |p| {
                let state = world.state(p);
                if state.is_none() && (0..CHUNK_HEIGHT).contains(&p.y) {
                    missing = Some(crate::voxel::split_world_position(p).0);
                }
                state
            })
        } else {
            state
        };
        let support = if reasons & NEIGHBOR_SUPPORT != 0 {
            crate::block_behavior::supports::evaluate(position, state, |p| world.state(p))
        } else {
            Ok(None)
        };
        if let Err(chunk) = &support {
            missing = Some(*chunk);
        }
        if let Some(chunk) = missing {
            work.suspended.insert(
                (cell, PendingChannel::Neighbor),
                BoundaryWork {
                    chunk: chunk.to_array(),
                    cell,
                    reasons,
                    tick: None,
                },
            );
            waits.register(chunk, owner);
            continue;
        }
        if reasons & NEIGHBOR_FLUID != 0 {
            let family = fluids.info(state).family;
            if family != hollow_core::FluidId::NONE && fluids.profile(family).automatic {
                work.schedule(
                    cell,
                    TickChannel::Fluid,
                    u32::from(family.0),
                    fluids.profile(family).delay,
                    0,
                );
            }
        }
        if let Ok(Some(edit)) = support {
            // The player observer also performs loot and inventory bookkeeping.
            // Atomic support pairs reserve their full quota before being submitted.
            if edit.edits.iter().any(|edit| {
                let chunk = crate::voxel::split_world_position(edit.position).0;
                chunk != column.position
            }) {
                work.enqueue(cell, reasons);
                break;
            }
            budget.mutations -= edit.edits.len();
            diagnostics.modified_cells += edit.edits.len();
            commands.trigger(edit);
        } else if next != state {
            let _ = world.commit(
                &[CheckedEdit {
                    position,
                    expected: state,
                    state: next,
                }],
                &mut commands,
            );
            budget.mutations -= 1;
            diagnostics.modified_cells += 1;
        }
    }
}

/// Lazily allocated registry: M1 registers no growth, fire or decay behavior.
type RandomHandler = fn(IVec3, VoxelState, &mut Commands);
#[derive(Resource, Default)]
pub(crate) struct RandomTickRegistry {
    handlers: Option<Box<[Option<RandomHandler>]>>,
}
impl RandomTickRegistry {
    #[allow(dead_code)]
    pub(crate) fn register(&mut self, state: VoxelState, handler: RandomHandler) {
        self.handlers
            .get_or_insert_with(|| vec![None; VoxelState::count()].into_boxed_slice())
            [state.id() as usize] = Some(handler);
    }
}
pub(crate) fn random_ticks(
    registry: Res<RandomTickRegistry>,
    clock: Res<GameplayClock>,
    settings: Res<crate::voxel::VoxelSettings>,
    terrain: Query<(&LoadedColumn, Ref<crate::voxel::ChunkVoxels>)>,
    mut columns: Query<
        (
            Entity,
            &GameplayColumn,
            &mut ColumnSimulation,
            Has<RandomEligibilityDirty>,
        ),
        With<Simulating>,
    >,
    mut budget: ResMut<SimulationBudget>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut commands: Commands,
) {
    let Some(handlers) = &registry.handlers else {
        return;
    };
    for (owner, voxels) in &terrain {
        let Ok((entity, column, mut work, dirty)) = columns.get_mut(owner.0) else {
            continue;
        };
        let mut sections = if registry.is_changed() || dirty || work.is_added() {
            hollow_core::coordinates::ALL_SECTIONS
        } else {
            work.random_dirty
        };
        work.random_dirty = 0;
        if dirty {
            commands.entity(entity).remove::<RandomEligibilityDirty>();
        }
        while sections != 0 {
            let section = sections.trailing_zeros() as usize;
            sections &= sections - 1;
            work.random_sections &= !(1 << section);
            if voxels.states.section(section).any(|state| {
                handlers
                    .get(state.id() as usize)
                    .copied()
                    .flatten()
                    .is_some()
            }) {
                work.random_sections |= 1 << section;
            }
        }
        let mut mask = work.random_sections;
        while mask != 0 {
            let section = mask.trailing_zeros();
            mask &= mask - 1;
            for attempt in 0..3 {
                if budget.random == 0 {
                    return;
                }
                budget.random -= 1;
                diagnostics.random_attempts += 1;
                let key = mix(settings.seed as u64
                    ^ mix(column.position.x as u64)
                    ^ mix((column.position.y as u64).rotate_left(32))
                    ^ mix(clock.0)
                    ^ mix(u64::from(section) * 3 + attempt));
                let cell = section * 4096 + (key as u32 & 4095);
                let position = cell_position(column.position, cell);
                let state = voxels.states.get(cell as usize);
                if let Some(handler) = handlers.get(state.id() as usize).copied().flatten() {
                    handler(position, state, &mut commands);
                }
            }
        }
    }
}
pub(super) fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
