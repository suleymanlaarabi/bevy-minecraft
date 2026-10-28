//! Persistent bounded voxel work and shared simulation scheduling.
use bevy_app::{App, FixedUpdate, Plugin, RunFixedMainLoop};
use bevy_derive::Deref;
use bevy_ecs::prelude::*;
use bevy_math::IVec2;
use bevy_platform::collections::HashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub const NEIGHBOR_CONNECTION: u8 = 1;
pub const NEIGHBOR_SUPPORT: u8 = 2;
pub const NEIGHBOR_FLUID: u8 = 4;
pub const NEIGHBOR_ALL: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TickChannel {
    Block,
    Fluid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScheduledTick {
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
pub struct ColumnSimulation {
    pub local_tick: u64,
    order: u64,
    scheduled: Option<Box<ScheduledWork>>,
    // Usually only a few sections have work. A sparse tree avoids
    // reserving an array of vector-bearing entries on every otherwise idle owner.
    neighbors: BTreeMap<u8, NeighborSection>,
    // Frontier keys are sparse. A direct cell/channel table would reserve several
    // MiB per owner even when no destination is blocked.
    pub suspended: BTreeMap<(u32, PendingChannel), BoundaryWork>,
    neighbor_count: usize,
    pub random_sections: u32,
    pub random_dirty: u32,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PendingChannel {
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
pub struct BoundaryWork {
    pub chunk: [i32; 2],
    pub cell: u32,
    pub reasons: u8,
    pub tick: Option<ScheduledTick>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SimulationSnapshot {
    local_tick: u64,
    order: u64,
    scheduled: Vec<ScheduledTick>,
    neighbors: Vec<(u32, u8)>,
    suspended: Vec<BoundaryWork>,
}
impl ColumnSimulation {
    pub fn wake(&mut self, chunk: IVec2, ready: &mut Vec<BoundaryWork>) {
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

    pub fn invalidate(&mut self, cell: u32, block: u32, fluid: u32) {
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
    pub fn defer_tick(&mut self, tick: ScheduledTick) {
        self.scheduled.get_or_insert_default().insert(tick);
    }
    pub fn invalidate_random_sections(&mut self, sections: u32) {
        self.random_dirty |= sections;
    }
    pub fn snapshot_bytes(&self) -> usize {
        self.scheduled.as_ref().map_or(0, |work| work.cells.len())
            * std::mem::size_of::<ScheduledTick>()
            + self.neighbor_count * std::mem::size_of::<(u32, u8)>()
            + self.suspended.len() * std::mem::size_of::<BoundaryWork>()
    }
    pub fn capture_reservation(&self) -> usize {
        // Include compact copies, JSON vector growth, compression input/output
        // buffers and allocator headroom, rather than counting tree handles alone.
        4096 + self.scheduled.as_ref().map_or(0, |work| work.cells.len()) * 1024
            + self.neighbor_count * 128
            + self.suspended.len() * 2048
    }
    pub fn estimated_bytes(&self) -> usize {
        // Charge shared allocations in full: capture/encoding also retain them.
        self.neighbors
            .values()
            .map(|section| std::mem::size_of::<NeighborSection>() + section.heap_bytes() + 512)
            .sum::<usize>()
            + self.scheduled.as_ref().map_or(0, |work| {
                work.cells.len() * (2 * std::mem::size_of::<ScheduledTick>() + 128)
            })
            + self.suspended.len() * (std::mem::size_of::<BoundaryWork>() + 128)
    }
    pub fn ready_count(&self) -> usize {
        self.neighbor_count + usize::from(self.has_due())
    }
    pub fn has_neighbors(&self) -> bool {
        !self.neighbors.is_empty()
    }
    pub fn has_due(&self) -> bool {
        self.scheduled
            .as_ref()
            .and_then(|work| work.ordered.first())
            .is_some_and(|tick| tick.due <= self.local_tick)
    }
    pub fn pop_tick(&mut self) -> Option<ScheduledTick> {
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
    pub fn suspend_tick(&mut self, chunk: IVec2, tick: ScheduledTick) {
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
    pub fn enqueue(&mut self, cell: u32, reasons: u8) {
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
    pub fn schedule(
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
    pub fn pop_neighbor(&mut self) -> Option<(u32, u8)> {
        let mut section = self.neighbors.first_entry()?;
        let (cell, reasons) = section.get_mut().pop()?;
        let index = u32::from(*section.key()) * 4096 + u32::from(cell);
        if section.get().is_empty() {
            section.remove();
        }
        self.neighbor_count -= 1;
        Some((index, reasons))
    }
    pub fn has_work(&self) -> bool {
        self.scheduled
            .as_ref()
            .is_some_and(|work| !work.ordered.is_empty())
            || !self.neighbors.is_empty()
            || !self.suspended.is_empty()
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
    pub fn restore(
        snapshot: SimulationSnapshot,
        section_count: usize,
    ) -> Result<Self, std::io::Error> {
        let mut work = Self {
            local_tick: snapshot.local_tick,
            order: snapshot.order,
            ..Default::default()
        };
        if !(1..=32).contains(&section_count) {
            return Err(std::io::Error::other("Invalid column section count"));
        }
        let valid = |cell| cell < section_count as u32 * 4096;
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
pub struct BoundaryWaits(HashMap<IVec2, Vec<Entity>>);
impl BoundaryWaits {
    pub fn remove_owner(&mut self, owner: Entity) {
        self.0.retain(|_, owners| {
            owners.retain(|entity| *entity != owner);
            !owners.is_empty()
        });
    }
    pub fn wake(&mut self, chunk: IVec2, ready: &mut Vec<(IVec2, Entity)>) {
        ready.extend(
            self.0
                .remove(&chunk)
                .into_iter()
                .flatten()
                .map(|owner| (chunk, owner)),
        );
    }
    pub fn register(&mut self, chunk: IVec2, owner: Entity) {
        let owners = self.0.entry(chunk).or_default();
        if !owners.contains(&owner) {
            owners.push(owner);
        }
    }
}
#[derive(Resource, Default, Clone, Copy)]
pub struct GameplayClock(pub u64);

/// Suspend fixed gameplay while leaving terrain streaming and loading operational.
#[derive(Resource)]
pub struct SimulationPaused;

#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct Simulating;

#[derive(Component)]
#[relationship(relationship_target = SimulatedEntities)]
pub struct SimulatedIn(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = SimulatedIn)]
pub struct SimulatedEntities(Vec<Entity>);

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimulationSystems {
    Activation,
    Neighbors,
    Scheduled,
    Random,
    Machines,
    Actors,
    Finalize,
}

#[derive(Resource, Default, PartialEq, Eq)]
pub struct SimulationRegion {
    pub center: Option<IVec2>,
    pub radius: u32,
}
impl SimulationRegion {
    pub fn contains(&self, position: IVec2) -> bool {
        self.center.is_some_and(|center| {
            hollow_core::coordinates::chunk_distance_squared(position, center)
                <= (self.radius * self.radius) as i32
        })
    }
}

#[derive(Resource, Default)]
pub struct SimulationDiagnostics {
    pub active_columns: usize,
    pub neighbor_cells: usize,
    pub scheduled_ticks: usize,
    pub modified_cells: usize,
    pub random_attempts: usize,
    pub ready_work: usize,
    pub suspended_work: usize,
    pub pending_write_bytes: usize,
    pub maximum_lateness: u64,
}
#[derive(Resource, Default)]
pub struct SimulationBudget {
    pub neighbors: usize,
    pub scheduled: usize,
    pub mutations: usize,
    pub random: usize,
}
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct ReadyTicks;
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct ReadyNeighbors;
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct RandomEligibilityDirty;
pub fn advance_clock(
    mut commands: Commands,
    mut clock: ResMut<GameplayClock>,
    mut columns: Query<
        (
            Entity,
            &mut ColumnSimulation,
            Has<ReadyTicks>,
            Has<ReadyNeighbors>,
        ),
        With<Simulating>,
    >,
) {
    clock.0 = clock.0.checked_add(1).expect("Gameplay clock exhausted");
    for (entity, mut column, ticks, neighbors) in &mut columns {
        column.local_tick = column
            .local_tick
            .checked_add(1)
            .expect("Column clock exhausted");
        if column.has_due() != ticks {
            if ticks {
                commands.entity(entity).remove::<ReadyTicks>();
            } else {
                commands.entity(entity).insert(ReadyTicks);
            }
        }
        if column.has_neighbors() != neighbors {
            if neighbors {
                commands.entity(entity).remove::<ReadyNeighbors>();
            } else {
                commands.entity(entity).insert(ReadyNeighbors);
            }
        }
    }
}
fn reset_budget(
    mut budget: ResMut<SimulationBudget>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
) {
    *budget = SimulationBudget {
        neighbors: 4096,
        scheduled: 2048,
        mutations: 4096,
        random: 8192,
    };
    diagnostics.neighbor_cells = 0;
    diagnostics.scheduled_ticks = 0;
    diagnostics.modified_cells = 0;
    diagnostics.random_attempts = 0;
}

pub struct SimulationPlugin;
impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameplayClock>()
            .add_observer(reset_session)
            .add_observer(remove_work)
            .init_resource::<SimulationRegion>()
            .init_resource::<SimulationBudget>()
            .init_resource::<SimulationDiagnostics>()
            .init_resource::<BoundaryWaits>()
            .configure_sets(
                FixedUpdate,
                (
                    SimulationSystems::Activation,
                    SimulationSystems::Neighbors,
                    SimulationSystems::Scheduled,
                    SimulationSystems::Random,
                    SimulationSystems::Machines,
                    SimulationSystems::Actors,
                    SimulationSystems::Finalize,
                )
                    .chain(),
            )
            .add_systems(
                RunFixedMainLoop,
                reset_budget.in_set(bevy_app::RunFixedMainLoopSystems::BeforeFixedMainLoop),
            );
    }
}

/// Durable owner metadata and application state share one contiguous component.
/// The engine never inspects or serializes the application payload.
#[derive(Component)]
#[require(ColumnSimulation)]
pub struct ColumnData<A: Send + Sync + 'static = ()> {
    pub position: IVec2,
    pub revision: u64,
    pub saved_revision: u64,
    pub initialized: bool,
    pub application: A,
}
#[derive(Resource, Default)]
pub struct SimulationSeed(pub i64);
pub mod runtime;

fn reset_session(
    _: On<crate::session::SessionClosed>,
    mut clock: ResMut<GameplayClock>,
    mut waits: ResMut<BoundaryWaits>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut region: ResMut<SimulationRegion>,
    mut commands: Commands,
) {
    *region = Default::default();
    commands.remove_resource::<SimulationPaused>();
    *clock = Default::default();
    *waits = Default::default();
    *diagnostics = Default::default();
}

fn remove_work(event: On<Remove<ColumnSimulation>>, mut waits: ResMut<BoundaryWaits>) {
    waits.remove_owner(event.entity);
}
