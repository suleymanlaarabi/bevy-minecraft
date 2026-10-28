//! Shared voxel scheduling; content policy is statically dispatched at work boundaries.
use super::*;
use crate::chunk::VoxelChunk;
use crate::{
    blocks::{BlockEntities, ColumnOwners, LoadedColumn},
    edits::{CheckedEdit, VoxelEditWorld},
    fluids::{FluidCatalog, FluidScratch, FluidWorld},
    world::{ChunkIndex, WorldLayout},
};
use bevy_ecs::system::SystemParam;
use bevy_math::{IVec3, Vec3};
use hollow_core::{FluidId, VoxelCell};
use std::marker::PhantomData;
#[derive(Event)]
pub struct FluidDisplaced<T: VoxelCell> {
    pub position: IVec3,
    pub state: T,
}
pub trait NeighborRules: Resource {
    type Cell: VoxelCell;
    type Application: Send + Sync + 'static;
    type Support: for<'a> Event<Trigger<'a> = bevy_ecs::event::GlobalTrigger>;
    fn connected(
        &self,
        position: IVec3,
        state: Self::Cell,
        sample: impl FnMut(IVec3) -> Option<Self::Cell>,
    ) -> Self::Cell;
    fn support(
        &self,
        position: IVec3,
        state: Self::Cell,
        sample: impl FnMut(IVec3) -> Option<Self::Cell>,
    ) -> Result<Option<Self::Support>, IVec2>;
    fn edits(edit: &Self::Support) -> &[CheckedEdit<Self::Cell>];
}
// Readiness is re-evaluated only after membership, terrain or ownership changes.
pub fn activate<A: Send + Sync + 'static>(
    mut commands: Commands,
    index: Res<ChunkIndex>,
    region: Res<SimulationRegion>,
    terrain: Query<(&VoxelChunk, &LoadedColumn)>,
    mut columns: Query<(
        &ColumnData<A>,
        Option<&BlockEntities>,
        Option<&SimulatedEntities>,
        Has<Simulating>,
        &mut ColumnSimulation,
    )>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut waits: ResMut<BoundaryWaits>,
    mut wake_owners: Local<Vec<(IVec2, Entity)>>,
    mut ready_work: Local<Vec<BoundaryWork>>,
) {
    if !region.is_changed() && !index.is_changed() {
        return;
    }
    let mut active = 0;
    for (chunk, owner) in &terrain {
        let Ok((column, blocks, mobiles, was_active, work)) = columns.get(owner.0) else {
            continue;
        };
        let ready = column.initialized
            && region.contains(chunk.position)
            && (-1..=1).all(|x| {
                (-1..=1).all(|z| index.contains_key(&(chunk.position + IVec2::new(x, z))))
            });
        active += usize::from(ready);
        if was_active == ready {
            continue;
        }
        for entity in std::iter::once(owner.0)
            .chain(blocks.into_iter().flat_map(|entities| entities.iter()))
            .chain(mobiles.into_iter().flat_map(|entities| entities.iter()))
        {
            if ready {
                commands.entity(entity).insert(Simulating);
            } else {
                commands.entity(entity).remove::<Simulating>();
            }
        }
        if ready {
            commands.entity(owner.0).insert(RandomEligibilityDirty);
            waits.wake(chunk.position, &mut wake_owners);
            for task in work.suspended.values() {
                let dependency = IVec2::from_array(task.chunk);
                if let Some(&terrain_entity) = index.get(&dependency)
                    && let Ok((_, target)) = terrain.get(terrain_entity)
                    && columns
                        .get(target.0)
                        .is_ok_and(|(_, _, _, active, _)| active)
                {
                    waits.wake(dependency, &mut wake_owners);
                }
            }
        }
    }
    for (chunk, owner) in wake_owners.drain(..) {
        if let Ok((_, _, _, _, mut work)) = columns.get_mut(owner) {
            work.wake(chunk, &mut ready_work);
        }
    }
    diagnostics.active_columns = active;
}
/// Neighbor scheduling borrows only the terrain and per-column work it touches.
#[derive(SystemParam)]
pub struct NeighborUpdates<'w, 's, T: VoxelCell> {
    terrain: crate::world::VoxelWorld<'w, 's, T>,
    owners: Res<'w, ColumnOwners>,
    work: Query<'w, 's, &'static mut ColumnSimulation>,
    fluids: Res<'w, crate::fluids::FluidCatalog>,
}
impl<T: VoxelCell> NeighborUpdates<'_, '_, T> {
    pub fn enqueue(&mut self, position: IVec3) {
        let center = self.terrain.layout().split(position).0;
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
            let (chunk, local) = self.terrain.layout().split(target);
            let Some(cell) = hollow_core::coordinates::column_cell_index(
                local,
                self.terrain.layout().section_count(),
            ) else {
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
                        self.fluids.definition(state).block.0,
                        u32::from(self.fluids.info(state).family.0),
                    );
                }
                work.invalidate_random_sections(1 << (cell / 4096));
            }
            work.enqueue(cell as u32, NEIGHBOR_ALL);
        }
    }
}
pub fn cell_position(chunk: IVec2, cell: u32, min_y: i32) -> IVec3 {
    IVec3::new(
        chunk.x * 16 + (cell & 15) as i32,
        min_y + (cell >> 8) as i32,
        chunk.y * 16 + ((cell >> 4) & 15) as i32,
    )
}

pub fn process_neighbors<P: NeighborRules>(
    mut commands: Commands,
    mut world: VoxelEditWorld<P::Cell>,
    rules: Res<P>,
    layout: Res<WorldLayout>,
    mut columns: Query<
        (Entity, &ColumnData<P::Application>, &mut ColumnSimulation),
        (With<Simulating>, With<ReadyNeighbors>),
    >,
    mut budget: ResMut<SimulationBudget>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut waits: ResMut<BoundaryWaits>,
    fluids: Res<crate::fluids::FluidCatalog>,
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
        let position = cell_position(column.position, cell, layout.min_y());
        let Some(state) = world.state(position) else {
            continue;
        };
        let mut missing = None;
        let next = if reasons & NEIGHBOR_CONNECTION != 0 {
            rules.connected(position, state, |p| {
                let state = world.state(p);
                if state.is_none() && layout.contains(p.y) {
                    missing = Some(layout.split(p).0);
                }
                state
            })
        } else {
            state
        };
        let support = if reasons & NEIGHBOR_SUPPORT != 0 {
            rules.support(position, state, |p| world.state(p))
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
            if P::edits(&edit).iter().any(|edit| {
                let chunk = layout.split(edit.position).0;
                chunk != column.position
            }) {
                work.enqueue(cell, reasons);
                break;
            }
            budget.mutations -= P::edits(&edit).len();
            diagnostics.modified_cells += P::edits(&edit).len();
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
type RandomHandler<T> = fn(IVec3, T, &mut Commands);
#[derive(Resource)]
pub struct RandomTickRegistry<T: VoxelCell> {
    handlers: Option<Box<[Option<RandomHandler<T>>]>>,
}
impl<T: VoxelCell> Default for RandomTickRegistry<T> {
    fn default() -> Self {
        Self { handlers: None }
    }
}
impl<T: VoxelCell> RandomTickRegistry<T> {
    #[allow(dead_code)]
    pub fn register(
        &mut self,
        state: T,
        handler: RandomHandler<T>,
        content: &hollow_core::content::ContentCatalog,
    ) {
        self.handlers
            .get_or_insert_with(|| vec![None; content.state_count()].into_boxed_slice())
            [state.id() as usize] = Some(handler);
    }
}
pub fn random_ticks<P: NeighborRules>(
    registry: Res<RandomTickRegistry<P::Cell>>,
    clock: Res<GameplayClock>,
    settings: Res<SimulationSeed>,
    layout: Res<WorldLayout>,
    terrain: Query<(&LoadedColumn, Ref<crate::chunk::ChunkVoxels<P::Cell>>)>,
    mut columns: Query<
        (
            Entity,
            &ColumnData<P::Application>,
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
            layout.mask()
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
                let key = mix(settings.0 as u64
                    ^ mix(column.position.x as u64)
                    ^ mix((column.position.y as u64).rotate_left(32))
                    ^ mix(clock.0)
                    ^ mix(u64::from(section) * 3 + attempt));
                let cell = section * 4096 + (key as u32 & 4095);
                let position = cell_position(column.position, cell, layout.min_y());
                let state = voxels.states.get(cell as usize);
                if let Some(handler) = handlers.get(state.id() as usize).copied().flatten() {
                    handler(position, state, &mut commands);
                }
            }
        }
    }
}
pub fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
struct FlowWorld<'a, 'w, 's, T: VoxelCell> {
    world: &'a VoxelEditWorld<'w, 's, T>,
    origin: IVec2,
    terrain: [Option<Entity>; 9],
    owners: &'a ColumnOwners,
    active: &'a Query<'w, 's, (), With<Simulating>>,
}
impl<T: VoxelCell> FluidWorld for FlowWorld<'_, '_, '_, T> {
    type Cell = T;
    fn vertical_bounds(&self) -> std::ops::Range<i32> {
        self.world.layout().min_y()..self.world.layout().max_y()
    }
    fn read(&self, position: IVec3) -> Result<T, IVec2> {
        let (chunk, local) = self.world.layout().split(position);
        let offset = chunk - self.origin;
        let state = if offset.abs().max_element() <= 1 {
            self.terrain[(offset.x + 1 + (offset.y + 1) * 3) as usize]
                .and_then(|entity| self.world.chunk_state(entity, local))
        } else {
            self.world.state(position)
        };
        state.ok_or(chunk)
    }
    fn shape_offset(&self, state: T, position: IVec3) -> Option<Vec3> {
        state.shape_offset(position)
    }
    fn writable(&self, position: IVec3) -> Result<(), IVec2> {
        let chunk = self.world.layout().split(position).0;
        if self
            .owners
            .get(&chunk)
            .is_some_and(|&owner| self.active.contains(owner))
        {
            Ok(())
        } else {
            Err(chunk)
        }
    }
}
pub fn process_scheduled<P: NeighborRules>(
    mut commands: Commands,
    mut world: VoxelEditWorld<P::Cell>,
    layout: Res<WorldLayout>,
    mut columns: Query<
        (Entity, &ColumnData<P::Application>, &mut ColumnSimulation),
        (With<Simulating>, With<ReadyTicks>),
    >,
    owners: Res<ColumnOwners>,
    active: Query<(), With<Simulating>>,
    settings: Res<SimulationSeed>,
    catalog: Res<FluidCatalog>,
    mut scratch: Local<FluidScratch<P::Cell>>,
    mut budget: ResMut<SimulationBudget>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut waits: ResMut<BoundaryWaits>,
    // Store the sort key once rather than fetching the column for each comparison.
    mut order: Local<Vec<((i32, i32), Entity)>>,
    mut cursor: Local<usize>,
) {
    order.clear();
    order.extend(
        columns
            .iter()
            .filter(|(_, _, work)| work.has_due())
            .map(|(entity, column, _)| ((column.position.x, column.position.y), entity)),
    );
    order.sort_unstable_by_key(|&(position, _)| position);
    if order.is_empty() {
        return;
    }
    let mut idle = 0;
    while budget.scheduled > 0 && idle < order.len() {
        *cursor %= order.len();
        let (_, owner) = order[*cursor];
        *cursor += 1;
        let Ok((_, column, mut work)) = columns.get_mut(owner) else {
            continue;
        };
        let Some(tick) = work.pop_tick() else {
            idle += 1;
            continue;
        };
        idle = 0;
        budget.scheduled -= 1;
        diagnostics.scheduled_ticks += 1;
        diagnostics.maximum_lateness = diagnostics
            .maximum_lateness
            .max(work.local_tick.saturating_sub(tick.due));
        let position = cell_position(column.position, tick.cell, layout.min_y());
        let Some(state) = world.state(position) else {
            continue;
        };
        let fluid = catalog.info(state);
        if tick.channel != TickChannel::Fluid
            || fluid.family == FluidId::NONE
            || u32::from(fluid.family.0) != tick.expected
            || !catalog.profile(fluid.family).automatic
        {
            continue;
        }
        scratch.edits.clear();
        let context = FlowWorld {
            world: &world,
            origin: column.position,
            // Slope searches stay in this 3x3 neighborhood. Resolve sparse chunk
            // coordinates once, then sample by entity ID without repeated hashing.
            terrain: std::array::from_fn(|i| {
                world.chunk_entity(
                    column.position + IVec2::new((i % 3) as i32 - 1, (i / 3) as i32 - 1),
                )
            }),
            owners: &owners,
            active: &active,
        };
        match crate::fluids::step(&catalog, &context, position, &mut scratch) {
            Err(chunk) => {
                work.suspend_tick(chunk, tick);
                waits.register(chunk, owner);
            }
            Ok(()) => {
                let count = scratch.edits.len();
                if count > budget.mutations {
                    work.defer_tick(tick);
                    break;
                }
                if world.commit(&scratch.edits, &mut commands).is_ok() {
                    if let Some(edit) = scratch.edits.iter().find(|edit| edit.position == position)
                    {
                        let next = catalog.info(edit.state);
                        if next.family != FluidId::NONE && edit.expected != edit.state {
                            let slow = catalog.profile(fluid.family).rising_delay_chance > 0
                                && !fluid.falling
                                && !next.falling
                                && next.amount > fluid.amount
                                && mix(settings.0 as u64 ^ work.local_tick ^ u64::from(tick.cell))
                                    % u64::from(
                                        catalog.profile(fluid.family).rising_delay_chance.max(1),
                                    )
                                    != 0;
                            work.schedule(
                                tick.cell,
                                TickChannel::Fluid,
                                u32::from(next.family.0),
                                catalog.profile(next.family).delay
                                    * if slow {
                                        u64::from(
                                            catalog.profile(next.family).rising_delay_multiplier,
                                        )
                                    } else {
                                        1
                                    },
                                tick.priority,
                            );
                        }
                    }
                    budget.mutations -= count;
                    diagnostics.modified_cells += count;
                    if catalog.profile(fluid.family).drops_displaced {
                        for edit in &scratch.edits {
                            if !catalog.definition(edit.expected).is_air()
                                && catalog.info(edit.expected).family == FluidId::NONE
                                && !catalog.info(edit.expected).container
                                && catalog.info(edit.state).family != FluidId::NONE
                            {
                                commands.trigger(FluidDisplaced {
                                    position: edit.position,
                                    state: edit.expected,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

pub struct VoxelSimulationPlugin<P: NeighborRules>(PhantomData<P>);
impl<P: NeighborRules> Default for VoxelSimulationPlugin<P> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<P: NeighborRules> Plugin for VoxelSimulationPlugin<P> {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<SimulationPlugin>() {
            app.add_plugins(SimulationPlugin);
        }
        app.init_resource::<ColumnOwners>()
            .init_resource::<SimulationSeed>()
            .init_resource::<RandomTickRegistry<P::Cell>>()
            .add_systems(
                FixedUpdate,
                (activate::<P::Application>, advance_clock)
                    .chain()
                    .in_set(SimulationSystems::Activation),
            )
            .add_systems(
                FixedUpdate,
                process_neighbors::<P>.in_set(SimulationSystems::Neighbors),
            )
            .add_systems(
                FixedUpdate,
                process_scheduled::<P>.in_set(SimulationSystems::Scheduled),
            )
            .add_systems(
                FixedUpdate,
                random_ticks::<P>.in_set(SimulationSystems::Random),
            );
    }
}
