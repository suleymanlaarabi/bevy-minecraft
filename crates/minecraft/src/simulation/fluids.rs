//! Budgeted scheduling adapter. All flow and reaction rules are engine services.
use super::*;
use crate::voxel::{VoxelState, edits::VoxelEditWorld};
use hollow_core::FluidId;
pub(crate) use hollow_engine::fluids::FluidCatalog;
use hollow_engine::fluids::{FluidScratch, FluidWorld};

struct FlowWorld<'a, 'w, 's> {
    world: &'a VoxelEditWorld<'w, 's>,
    origin: IVec2,
    terrain: [Option<Entity>; 9],
    owners: &'a GameplayColumns,
    active: &'a Query<'w, 's, (), With<Simulating>>,
}
impl FluidWorld for FlowWorld<'_, '_, '_> {
    type Cell = VoxelState;
    fn read(&self, position: IVec3) -> Result<VoxelState, IVec2> {
        let (chunk, local) = crate::voxel::split_world_position(position);
        let offset = chunk - self.origin;
        let state = if offset.abs().max_element() <= 1 {
            self.terrain[(offset.x + 1 + (offset.y + 1) * 3) as usize]
                .and_then(|entity| self.world.chunk_state(entity, local))
        } else {
            self.world.state(position)
        };
        state.ok_or(chunk)
    }
    fn shape_offset(&self, state: VoxelState, position: IVec3) -> Option<Vec3> {
        state
            .shape_depends_on_position()
            .then(|| state.shape_offset(position))
    }
    fn writable(&self, position: IVec3) -> Result<(), IVec2> {
        let chunk = crate::voxel::split_world_position(position).0;
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
pub(crate) fn process_scheduled(
    mut commands: Commands,
    mut world: crate::voxel::edits::VoxelEditWorld,
    mut columns: Query<
        (Entity, &GameplayColumn, &mut ColumnSimulation),
        (With<Simulating>, With<ReadyTicks>),
    >,
    owners: Res<GameplayColumns>,
    active: Query<(), With<Simulating>>,
    settings: Res<crate::voxel::VoxelSettings>,
    catalog: Res<FluidCatalog>,
    mut scratch: Local<FluidScratch<VoxelState>>,
    mut budget: ResMut<SimulationBudget>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut waits: ResMut<BoundaryWaits>,
    // Store the sort key once rather than fetching the column for each comparison.
    mut order: Local<Vec<((i32, i32), Entity)>>,
    mut cursor: Local<usize>,
    mut random: ResMut<crate::block_behavior::loot::LootRandom>,
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
        let position = cell_position(column.position, tick.cell);
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
        match hollow_engine::fluids::step(&catalog, &context, position, &mut scratch) {
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
                                && super::work::mix(
                                    settings.seed as u64 ^ work.local_tick ^ u64::from(tick.cell),
                                ) % u64::from(
                                    catalog.profile(fluid.family).rising_delay_chance.max(1),
                                ) != 0;
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
                            if !edit.expected.definition().is_air()
                                && catalog.info(edit.expected).family == FluidId::NONE
                                && !catalog.info(edit.expected).container
                                && catalog.info(edit.state).family != FluidId::NONE
                            {
                                use crate::dropped_item::DroppedItemCommandsExt;
                                for stack in crate::block_behavior::loot::drops(
                                    edit.expected,
                                    false,
                                    &mut random,
                                )
                                .into_iter()
                                .flatten()
                                {
                                    commands.spawn_dropped_item(
                                        stack,
                                        edit.position.as_vec3() + Vec3::splat(0.5),
                                        Vec3::Y * 1.2,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
