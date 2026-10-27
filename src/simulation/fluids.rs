//! Flow rules transcribed from the local Minecraft 26.3 client classes:
//! FlowingFluid, WaterFluid, LavaFluid and LiquidBlock. Registry data supplies
//! identities and shapes; propagation timing is defined here, not by imported tuning.
use super::*;
use crate::voxel::{
    VoxelKind, VoxelState,
    edits::{CheckedEdit, VoxelEditWorld},
};
use pumpkin_data::{BlockId, Property, PropertyKey, tag::Taggable};

const SIDES: [IVec3; 4] = [IVec3::NEG_Z, IVec3::X, IVec3::Z, IVec3::NEG_X];
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(u16)]
pub(crate) enum FluidFamily {
    #[default]
    Empty,
    Water,
    Lava,
}
impl FluidFamily {
    pub(crate) fn delay(self) -> u64 {
        if self == Self::Lava { 30 } else { 5 }
    }
    fn drop_off(self) -> u8 {
        if self == Self::Lava { 2 } else { 1 }
    }
    fn slope_limit(self) -> u8 {
        if self == Self::Lava { 2 } else { 4 }
    }
}
#[derive(Clone, Copy, Default)]
pub(crate) struct FluidInfo {
    pub family: FluidFamily,
    amount: u8,
    source: bool,
    falling: bool,
    container: bool,
    receives_water: bool,
    washable: bool,
    filled: u16,
}
#[derive(Resource)]
pub(crate) struct FluidCatalog {
    states: Box<[FluidInfo]>,
    flowing: [[VoxelState; 16]; 2],
}
impl Default for FluidCatalog {
    fn default() -> Self {
        let states = (0..pumpkin_data::BlockStateId::COUNT)
            .map(|id| {
                let state = VoxelState::from_id(id).expect("Registry state");
                let kind = state.kind();
                let container = state.property(PropertyKey::Waterlogged).is_some();
                let family = if state.has_water() {
                    FluidFamily::Water
                } else if kind.0 == BlockId::LAVA {
                    FluidFamily::Lava
                } else {
                    FluidFamily::Empty
                };
                let level = if matches!(kind.0, BlockId::WATER | BlockId::LAVA) {
                    match state.property(PropertyKey::Level) {
                        Some(Property::Level(level)) => level,
                        _ => 0,
                    }
                } else {
                    0
                };
                FluidInfo {
                    family,
                    amount: if family == FluidFamily::Empty {
                        0
                    } else if level >= 8 {
                        8
                    } else {
                        8 - level
                    },
                    source: family != FluidFamily::Empty && level == 0,
                    falling: level >= 8,
                    container,
                    receives_water: container
                        && state.property(PropertyKey::Type)
                            != Some(Property::SlabType(
                                pumpkin_data::block_properties::SlabType::Double,
                            )),
                    washable: kind
                        .block()
                        .has_tag(&pumpkin_data::tag::Block::MINECRAFT_WASHED_AWAY_BY_FLUIDS),
                    filled: if container {
                        state.with(&[Property::Waterlogged(true)]).id()
                    } else {
                        id
                    },
                }
            })
            .collect();
        let flowing = [BlockId::WATER, BlockId::LAVA].map(|kind| {
            std::array::from_fn(|level| {
                VoxelKind(kind)
                    .default_state()
                    .with(&[Property::Level(level as u8)])
            })
        });
        Self { states, flowing }
    }
}
impl FluidCatalog {
    pub(crate) fn info(&self, state: VoxelState) -> FluidInfo {
        self.states[state.id() as usize]
    }
    pub(crate) fn reaction(
        &self,
        position: IVec3,
        state: VoxelState,
        mut read: impl FnMut(IVec3) -> Option<VoxelState>,
    ) -> Result<Option<VoxelState>, IVec2> {
        if self.info(state).family != FluidFamily::Lava {
            return Ok(None);
        }
        let mut sample = |p: IVec3| {
            if !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&p.y) {
                return Ok(VoxelKind::Air.default_state());
            }
            read(p).ok_or(crate::voxel::split_world_position(p).0)
        };
        let soil = sample(position - IVec3::Y)?.kind().0 == BlockId::SOUL_SOIL;
        for delta in SIDES.into_iter().chain([IVec3::Y]) {
            let neighbor = sample(position + delta)?;
            if self.info(neighbor).family == FluidFamily::Water {
                return Ok(Some(
                    VoxelKind(if self.info(state).source {
                        BlockId::OBSIDIAN
                    } else {
                        BlockId::COBBLESTONE
                    })
                    .default_state(),
                ));
            }
            if soil && neighbor.kind().0 == BlockId::BLUE_ICE {
                return Ok(Some(VoxelKind(BlockId::BASALT).default_state()));
            }
        }
        Ok(None)
    }
    fn liquid(&self, family: FluidFamily, amount: u8, falling: bool) -> VoxelState {
        if amount == 0 {
            return VoxelKind::Air.default_state();
        }
        let level = if falling { 8 } else { 8 - amount };
        self.flowing[family as usize - 1][level as usize]
    }
    fn holds(&self, state: VoxelState, family: FluidFamily) -> bool {
        let info = self.info(state);
        if info.container {
            family == FluidFamily::Water && info.family == FluidFamily::Empty && info.receives_water
        } else {
            info.washable
        }
    }
    fn destination(&self, state: VoxelState, fluid: VoxelState) -> VoxelState {
        let info = self.info(state);
        if info.container {
            VoxelState::from_id(info.filled).expect("Prepared waterlogged state")
        } else {
            fluid
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Passage {
    key: u64,
    valid: bool,
    open: bool,
}
pub(crate) struct FluidScratch {
    pub edits: Vec<CheckedEdit>,
    passages: [Passage; 512],
    rectangles: Vec<[f32; 4]>,
    axes: [Vec<f32>; 2],
    frontier: Vec<(IVec3, u8)>,
}
impl Default for FluidScratch {
    fn default() -> Self {
        Self {
            edits: Vec::with_capacity(6),
            passages: [Passage::default(); 512],
            rectangles: Vec::new(),
            axes: [Vec::new(), Vec::new()],
            frontier: Vec::with_capacity(81),
        }
    }
}
impl FluidScratch {
    // A fixed-size direct-mapped cache bounds memory regardless of registry size.
    // Shapes with offsets use the position-aware path and never enter this cache.
    fn passage(
        &mut self,
        source: VoxelState,
        source_pos: IVec3,
        target: VoxelState,
        delta: IVec3,
    ) -> bool {
        if source.full_collision() || target.full_collision() {
            return false;
        }
        if source.native().collision_shapes.is_empty()
            && target.native().collision_shapes.is_empty()
        {
            return true;
        }
        let axis = if delta.x != 0 {
            0
        } else if delta.y != 0 {
            1
        } else {
            2
        };
        let positive = delta[axis] > 0;
        let key = u64::from(source.id())
            | (u64::from(target.id()) << 16)
            | ((axis as u64 * 2 + u64::from(positive)) << 32);
        let slot = (key.wrapping_mul(0x9e3779b97f4a7c15) >> 55) as usize;
        // Offset blocks are plants; their collision data may depend on position.
        let cacheable = source.kind().block().shape_offset_limits() == (0.0, 0.0)
            && target.kind().block().shape_offset_limits() == (0.0, 0.0);
        if cacheable && self.passages[slot].valid && self.passages[slot].key == key {
            return self.passages[slot].open;
        }
        self.rectangles.clear();
        self.axes.iter_mut().for_each(|axis| {
            axis.clear();
            axis.extend([0.0, 1.0]);
        });
        let a = (axis + 1) % 3;
        let b = (axis + 2) % 3;
        for (state, pos, face) in [
            (source, source_pos, positive),
            (target, source_pos + delta, !positive),
        ] {
            for bounds in state.collision_boxes(pos) {
                let touches = if face {
                    bounds.min[axis] <= 1.0 && bounds.max[axis] >= 1.0
                } else {
                    bounds.min[axis] <= 0.0 && bounds.max[axis] >= 0.0
                };
                if !touches {
                    continue;
                }
                let rect = [
                    bounds.min[a].clamp(0.0, 1.0),
                    bounds.max[a].clamp(0.0, 1.0),
                    bounds.min[b].clamp(0.0, 1.0),
                    bounds.max[b].clamp(0.0, 1.0),
                ];
                self.axes[0].extend([rect[0], rect[1]]);
                self.axes[1].extend([rect[2], rect[3]]);
                self.rectangles.push(rect);
            }
        }
        for axis in &mut self.axes {
            axis.sort_unstable_by(f32::total_cmp);
            axis.dedup();
        }
        let open = self.axes[0].windows(2).any(|x| {
            self.axes[1].windows(2).any(|y| {
                let (x, y) = ((x[0] + x[1]) * 0.5, (y[0] + y[1]) * 0.5);
                !self
                    .rectangles
                    .iter()
                    .any(|r| x >= r[0] && x <= r[1] && y >= r[2] && y <= r[3])
            })
        });
        if cacheable {
            self.passages[slot] = Passage {
                key,
                valid: true,
                open,
            };
        }
        open
    }
}
struct FlowContext<'a, 'w, 's> {
    world: &'a VoxelEditWorld<'w, 's>,
    catalog: &'a FluidCatalog,
    origin: IVec2,
    terrain: [Option<Entity>; 9],
    owners: &'a GameplayColumns,
    active: &'a Query<'w, 's, (), With<Simulating>>,
}
impl FlowContext<'_, '_, '_> {
    fn read(&self, position: IVec3, scratch: &FluidScratch) -> Result<VoxelState, IVec2> {
        if !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&position.y) {
            return Ok(VoxelKind::Air.default_state());
        }
        if let Some(edit) = scratch.edits.iter().find(|edit| edit.position == position) {
            return Ok(edit.state);
        }
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
    fn write(
        &self,
        position: IVec3,
        state: VoxelState,
        scratch: &mut FluidScratch,
    ) -> Result<(), IVec2> {
        if !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&position.y) {
            return Ok(());
        }
        let chunk = crate::voxel::split_world_position(position).0;
        if !self
            .owners
            .get(&chunk)
            .is_some_and(|&owner| self.active.contains(owner))
        {
            return Err(chunk);
        }
        if let Some(edit) = scratch
            .edits
            .iter_mut()
            .find(|edit| edit.position == position)
        {
            edit.state = state;
        } else {
            let expected = self.read(position, scratch)?;
            if expected != state {
                scratch.edits.push(CheckedEdit {
                    position,
                    expected,
                    state,
                });
            }
        }
        Ok(())
    }
    fn new_liquid(
        &self,
        position: IVec3,
        family: FluidFamily,
        scratch: &mut FluidScratch,
    ) -> Result<VoxelState, IVec2> {
        let state = self.read(position, scratch)?;
        let (mut highest, mut sources) = (0, 0);
        for delta in SIDES {
            let neighbor = self.read(position + delta, scratch)?;
            let fluid = self.catalog.info(neighbor);
            if fluid.family == family && scratch.passage(state, position, neighbor, delta) {
                highest = highest.max(fluid.amount);
                sources += u8::from(fluid.source);
            }
        }
        if family == FluidFamily::Water && sources >= 2 {
            let below = self.read(position - IVec3::Y, scratch)?;
            let fluid = self.catalog.info(below);
            if below.native().is_solid() || (fluid.family == family && fluid.source) {
                return Ok(self.catalog.liquid(family, 8, false));
            }
        }
        let above = self.read(position + IVec3::Y, scratch)?;
        if self.catalog.info(above).family == family
            && scratch.passage(state, position, above, IVec3::Y)
        {
            return Ok(self.catalog.liquid(family, 8, true));
        }
        Ok(self
            .catalog
            .liquid(family, highest.saturating_sub(family.drop_off()), false))
    }
    fn hole(
        &self,
        position: IVec3,
        family: FluidFamily,
        scratch: &mut FluidScratch,
    ) -> Result<bool, IVec2> {
        if position.y <= 0 {
            return Ok(false);
        }
        let state = self.read(position, scratch)?;
        let below = self.read(position - IVec3::Y, scratch)?;
        Ok(scratch.passage(state, position, below, IVec3::NEG_Y)
            && (self.catalog.info(below).family == family || self.catalog.holds(below, family)))
    }
    fn slope(
        &self,
        position: IVec3,
        from: IVec3,
        family: FluidFamily,
        scratch: &mut FluidScratch,
    ) -> Result<u8, IVec2> {
        if self.hole(position, family, scratch)? {
            return Ok(0);
        }
        scratch.frontier.clear();
        scratch.frontier.push((position, 0));
        // At most four horizontal steps fit a 9x9 grid. A stack bitset avoids
        // retaining a coordinate vector and scanning it for every candidate.
        let visit_bit = |p: IVec3| {
            let offset = p - position;
            1u128 << ((offset.x + 4) + (offset.z + 4) * 9)
        };
        let mut visited = visit_bit(from) | visit_bit(position);
        let mut cursor = 0;
        while cursor < scratch.frontier.len() {
            let (pos, distance) = scratch.frontier[cursor];
            cursor += 1;
            if distance >= family.slope_limit() {
                continue;
            }
            let state = self.read(pos, scratch)?;
            for delta in SIDES {
                let next = pos + delta;
                let bit = visit_bit(next);
                if visited & bit != 0 {
                    continue;
                }
                let target = self.read(next, scratch)?;
                let fluid = self.catalog.info(target);
                if (fluid.family == family && fluid.source)
                    || !self.catalog.holds(target, family)
                    || !scratch.passage(state, pos, target, delta)
                {
                    continue;
                }
                visited |= bit;
                if self.hole(next, family, scratch)? {
                    return Ok(distance + 1);
                }
                scratch.frontier.push((next, distance + 1));
            }
        }
        Ok(u8::MAX)
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
    mut scratch: Local<FluidScratch>,
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
        if tick.channel != TickChannel::Fluid || fluid.family as u16 != tick.expected {
            continue;
        }
        scratch.edits.clear();
        let context = FlowContext {
            world: &world,
            catalog: &catalog,
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
        match flow(&context, position, state, &mut scratch) {
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
                if world.commit(&scratch.edits, &mut commands) {
                    if let Some(edit) = scratch.edits.iter().find(|edit| edit.position == position)
                    {
                        let next = catalog.info(edit.state);
                        if next.family != FluidFamily::Empty && edit.expected != edit.state {
                            let slow = fluid.family == FluidFamily::Lava
                                && !fluid.falling
                                && !next.falling
                                && next.amount > fluid.amount
                                && super::work::mix(
                                    settings.seed as u64 ^ work.local_tick ^ u64::from(tick.cell),
                                ) & 3
                                    != 0;
                            work.schedule(
                                tick.cell,
                                TickChannel::Fluid,
                                next.family as u16,
                                next.family.delay() * if slow { 4 } else { 1 },
                                tick.priority,
                            );
                        }
                    }
                    budget.mutations -= count;
                    diagnostics.modified_cells += count;
                    if fluid.family == FluidFamily::Water {
                        for edit in &scratch.edits {
                            if !edit.expected.native().is_air()
                                && catalog.info(edit.expected).family == FluidFamily::Empty
                                && !catalog.info(edit.expected).container
                                && catalog.info(edit.state).family != FluidFamily::Empty
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
fn flow(
    context: &FlowContext,
    position: IVec3,
    mut state: VoxelState,
    scratch: &mut FluidScratch,
) -> Result<(), IVec2> {
    let mut fluid = context.catalog.info(state);
    if fluid.family == FluidFamily::Empty {
        return Ok(());
    }
    let mut missing = None;
    let reaction = context
        .catalog
        .reaction(position, state, |p| match context.read(p, scratch) {
            Ok(state) => Some(state),
            Err(chunk) => {
                missing = Some(chunk);
                None
            }
        });
    if let Some(chunk) = missing {
        return Err(chunk);
    }
    if let Ok(Some(product)) = reaction {
        context.write(position, product, scratch)?;
        return Ok(());
    }
    if !fluid.source {
        let next = context.new_liquid(position, fluid.family, scratch)?;
        if state != next {
            context.write(position, next, scratch)?;
            state = next;
            fluid = context.catalog.info(state);
        }
        if fluid.family == FluidFamily::Empty {
            return Ok(());
        }
    }
    let mut down = false;
    if position.y > 0 {
        let below_pos = position - IVec3::Y;
        let below = context.read(below_pos, scratch)?;
        let target = context.catalog.info(below);
        if scratch.passage(state, position, below, IVec3::NEG_Y) {
            if fluid.family == FluidFamily::Water && target.family == FluidFamily::Lava {
                let product = if target.source {
                    BlockId::OBSIDIAN
                } else {
                    BlockId::COBBLESTONE
                };
                context.write(below_pos, VoxelKind(product).default_state(), scratch)?;
            }
            if fluid.family == FluidFamily::Lava
                && target.family == FluidFamily::Water
                && !target.container
                && context.catalog.holds(below, fluid.family)
            {
                if below.kind().0 == BlockId::WATER {
                    context.write(
                        below_pos,
                        VoxelKind(BlockId::STONE).default_state(),
                        scratch,
                    )?;
                }
                return Ok(());
            }
            if target.family == FluidFamily::Empty && context.catalog.holds(below, fluid.family) {
                let next = context.new_liquid(below_pos, fluid.family, scratch)?;
                let next = context.catalog.destination(below, next);
                context.write(below_pos, next, scratch)?;
                down = true;
            }
        }
    }
    let mut sources = 0;
    for delta in SIDES {
        let target = context
            .catalog
            .info(context.read(position + delta, scratch)?);
        sources += u8::from(target.family == fluid.family && target.source);
    }
    if down && sources < 3 {
        return Ok(());
    }
    if !down && !fluid.source && context.hole(position, fluid.family, scratch)? {
        return Ok(());
    }
    if !fluid.falling && fluid.amount <= fluid.family.drop_off() {
        return Ok(());
    }
    let mut choices = [None; 4];
    let mut lowest = u8::MAX;
    for (i, delta) in SIDES.into_iter().enumerate() {
        let target_pos = position + delta;
        let target = context.read(target_pos, scratch)?;
        let existing = context.catalog.info(target);
        if (existing.family == fluid.family && existing.source)
            || !context.catalog.holds(target, fluid.family)
            || !scratch.passage(state, position, target, delta)
        {
            continue;
        }
        let next = context.new_liquid(target_pos, fluid.family, scratch)?;
        if context.catalog.info(next).family == FluidFamily::Empty {
            continue;
        }
        let distance = context.slope(target_pos, position, fluid.family, scratch)?;
        if distance < lowest {
            choices.fill(None);
            lowest = distance;
        }
        if distance <= lowest {
            let replace = existing.family == FluidFamily::Empty
                || (existing.family == FluidFamily::Lava
                    && fluid.family == FluidFamily::Water
                    && existing.amount >= 4);
            if replace {
                choices[i] = Some(
                    if existing.family == FluidFamily::Lava && fluid.family == FluidFamily::Water {
                        VoxelKind(if existing.source {
                            BlockId::OBSIDIAN
                        } else {
                            BlockId::COBBLESTONE
                        })
                        .default_state()
                    } else {
                        context.catalog.destination(target, next)
                    },
                );
            }
        }
    }
    for (delta, next) in SIDES.into_iter().zip(choices) {
        if let Some(next) = next {
            context.write(position + delta, next, scratch)?;
        }
    }
    Ok(())
}
