//! Shared fluid propagation, shape passage and configurable contact reactions.
//! No Minecraft identities or rules belong in this module.
use crate::{
    content::{FreezeContent, RegisteredContent},
    edits::CheckedEdit,
};
use bevy_app::{App, Plugin, PreStartup};
use bevy_ecs::prelude::*;
use bevy_math::prelude::*;
use hollow_core::{
    BlockId, FluidId, ResourceId, StateId, VoxelCell,
    content::{ContentCatalog, StateDefinition},
    shape::BlockBox,
};
use std::sync::Arc;
const SIDES: [IVec3; 4] = [IVec3::NEG_Z, IVec3::X, IVec3::Z, IVec3::NEG_X];

/// A host physics system publishes this compact component. Content plugins can
/// implement arbitrary actor effects using normal queries/change detection.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct FluidContactState {
    pub fluid: FluidId,
    pub eyes: FluidId,
    pub depth: f32,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct FluidContact {
    pub swimmable: bool,
    pub extinguishes: bool,
    pub damage: f32,
    pub damage_resource: Option<ResourceId>,
    pub ignites: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct FluidMovement {
    pub acceleration: f64,
    pub horizontal_drag: f64,
    pub vertical_drag: f64,
    pub sprint_drag: f64,
    pub deep_vertical_drag: f64,
    pub gravity: f64,
    pub sprint_cancels_gravity: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct FluidFog {
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub ambient: [f32; 3],
    pub brightness: f32,
    pub daylight: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct FluidRender {
    pub still: ResourceId,
    pub flowing: ResourceId,
    pub translucent: bool,
    pub emission: u8,
    pub effect: u8,
    /// Opt into the renderer's shared ocean optics; other surfaces use explicit quads.
    pub ocean_surface: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum ContactFilter {
    Fluid(FluidId),
    Block(BlockId),
}
#[derive(Clone, Copy, Debug)]
pub enum ReactionTarget {
    Origin,
    Neighbor,
}
#[derive(Clone, Copy, Debug)]
pub struct FluidReaction {
    pub contact: ContactFilter,
    pub support: Option<BlockId>,
    /// Bits follow -Z, +X, +Z, -X, +Y, -Y.
    pub directions: u8,
    pub target: ReactionTarget,
    pub source_product: StateId,
    pub flowing_product: StateId,
    pub minimum_amount: u8,
    pub free_fluid_only: bool,
}
pub struct FluidProfile {
    pub name: String,
    pub delay: u64,
    pub drop_off: u8,
    pub slope_limit: u8,
    pub renewable_sources: Option<u8>,
    pub drops_displaced: bool,
    pub rising_delay_chance: u8,
    pub rising_delay_multiplier: u8,
    pub standing: Box<[StateId]>,
    pub falling: Box<[StateId]>,
    pub contact: FluidContact,
    pub movement: FluidMovement,
    pub fog: Option<FluidFog>,
    pub render: Option<FluidRender>,
    pub reactions: Vec<FluidReaction>,
    /// A plugin can own ticking through ordinary ECS systems while reusing the
    /// same catalog, contact, rendering and transaction services.
    pub automatic: bool,
    pub before_step: Option<Box<dyn FluidBehavior>>,
}
impl FluidProfile {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            delay: 5,
            drop_off: 1,
            slope_limit: 4,
            renewable_sources: None,
            drops_displaced: false,
            rising_delay_chance: 0,
            rising_delay_multiplier: 1,
            standing: Box::new([]),
            falling: Box::new([]),
            contact: FluidContact::default(),
            render: None,
            movement: FluidMovement {
                acceleration: 0.02,
                horizontal_drag: 0.8,
                vertical_drag: 0.8,
                sprint_drag: 0.8,
                deep_vertical_drag: 0.8,
                gravity: 0.005,
                sprint_cancels_gravity: false,
            },
            fog: None,
            reactions: Vec::new(),
            automatic: true,
            before_step: None,
        }
    }
}
impl FluidProfile {
    pub fn max_amount(&self) -> u8 {
        (self.standing.len() - 1) as u8
    }
}
#[derive(Resource)]
pub struct FluidRegistrations(Option<Vec<FluidProfile>>);
impl Default for FluidRegistrations {
    fn default() -> Self {
        Self(Some(Vec::new()))
    }
}
impl FluidRegistrations {
    /// Import boundary: retain the returned numeric ID in plugin configuration.
    pub fn resolve(&self, name: &str) -> Option<FluidId> {
        self.0
            .as_ref()
            .expect("Fluid registrations already frozen")
            .iter()
            .position(|profile| profile.name == name)
            .map(|i| FluidId((i + 1) as u16))
    }
    pub fn register(&mut self, profile: FluidProfile) -> FluidId {
        let profiles = self.0.as_mut().expect("Fluid registration already frozen");
        assert!(
            !profiles.iter().any(|p| p.name == profile.name),
            "Duplicate fluid name"
        );
        let id = FluidId(u16::try_from(profiles.len() + 1).expect("Too many fluids"));
        profiles.push(profile);
        id
    }
    /// Convenience for a levelled free fluid. Advanced providers can instead
    /// register arbitrary existing state families with register(profile).
    pub fn register_simple(
        &mut self,
        content: &mut hollow_core::content::ContentBuilder,
        mut block: hollow_core::content::BlockSpec,
        mut profile: FluidProfile,
        maximum: u8,
    ) -> (BlockId, FluidId) {
        use hollow_core::content::{FluidState, PropertySpec};
        assert!(
            maximum > 0 && block.properties.is_empty(),
            "Simple fluids declare their own amount/falling schema"
        );
        block.properties = vec![
            PropertySpec {
                name: "amount".into(),
                values: (1..=maximum).map(|n| n.to_string()).collect(),
                default: u16::from(maximum - 1),
            },
            PropertySpec {
                name: "falling".into(),
                values: vec!["false".into(), "true".into()],
                default: 0,
            },
        ];
        let block = content.register(block);
        let range = content.blocks[block.index()].states.clone();
        profile.standing = std::iter::once(StateId(range.start))
            .chain((1..=maximum).map(|n| StateId(range.start + u32::from(n - 1) * 2)))
            .collect();
        profile.falling = std::iter::once(StateId(range.start))
            .chain((1..=maximum).map(|n| StateId(range.start + u32::from(n - 1) * 2 + 1)))
            .collect();
        let id = self.register(profile);
        for raw in range {
            let index = raw - content.blocks[block.index()].states.start;
            let amount = (index / 2 + 1) as u8;
            let falling = index % 2 != 0;
            content.states[raw as usize].fluid = FluidState {
                id,
                amount,
                falling,
                source: !falling && amount == maximum,
                embedded: false,
            };
        }
        (block, id)
    }
    pub fn profile_mut(&mut self, id: FluidId) -> &mut FluidProfile {
        &mut self.0.as_mut().expect("Fluid registration already frozen")[id.index() - 1]
    }
}
struct FluidData {
    content: ContentCatalog,
    profiles: Box<[FluidProfile]>,
}
#[derive(Resource, Clone)]
pub struct FluidCatalog(Arc<FluidData>);
#[derive(Clone, Copy)]
pub struct FluidInfo {
    pub family: FluidId,
    pub amount: u8,
    pub source: bool,
    pub falling: bool,
    pub container: bool,
}
impl FluidCatalog {
    pub fn content(&self) -> &ContentCatalog {
        &self.0.content
    }
    pub fn profile(&self, id: FluidId) -> &FluidProfile {
        &self.0.profiles[id.index() - 1]
    }
    pub fn profiles(&self) -> impl Iterator<Item = (FluidId, &FluidProfile)> {
        self.0
            .profiles
            .iter()
            .enumerate()
            .map(|(i, p)| (FluidId((i + 1) as u16), p))
    }
    pub fn definition<T: VoxelCell>(&self, state: T) -> &StateDefinition {
        self.0.content.state(StateId(state.id()))
    }
    pub fn info<T: VoxelCell>(&self, state: T) -> FluidInfo {
        let definition = self.definition(state);
        let fluid = definition.fluid;
        FluidInfo {
            family: fluid.id,
            amount: fluid.amount,
            source: fluid.source,
            falling: fluid.falling,
            container: !definition.containers.is_empty() || fluid.embedded,
        }
    }
    fn shapes<T: VoxelCell>(&self, state: T) -> &[BlockBox] {
        self.0.content.shapes(self.definition(state).collision)
    }
    pub fn fill_height<T: VoxelCell>(&self, state: T) -> f64 {
        let fluid = self.info(state);
        if fluid.family == FluidId::NONE {
            0.0
        } else if fluid.falling {
            1.0
        } else {
            f64::from(fluid.amount) / (f64::from(self.profile(fluid.family).max_amount()) + 1.0)
        }
    }
    pub fn residual<T: VoxelCell>(&self, state: T) -> T {
        let family = self.info(state).family;
        if family == FluidId::NONE {
            T::air()
        } else {
            self.liquid(family, self.profile(family).max_amount(), false)
        }
    }
    fn liquid<T: VoxelCell>(&self, family: FluidId, amount: u8, falling: bool) -> T {
        if amount == 0 {
            return T::air();
        }
        let profile = self.profile(family);
        T::from_id(
            if falling {
                profile.falling[amount as usize].0
            } else {
                profile.standing[amount as usize].0
            },
            self.content(),
        )
        .expect("Registered fluid state")
    }
    fn holds<T: VoxelCell>(&self, state: T, family: FluidId) -> bool {
        let definition = self.definition(state);
        if !definition.containers.is_empty() {
            definition.fluid.id == FluidId::NONE
                && self
                    .0
                    .content
                    .container(StateId(state.id()), family)
                    .is_some()
        } else {
            definition.washable
        }
    }
    fn destination<T: VoxelCell>(&self, state: T, fluid: T) -> T {
        let family = self.info(fluid).family;
        self.0
            .content
            .container(StateId(state.id()), family)
            .map_or(fluid, |c| {
                T::from_id(c.filled.0, self.content()).expect("Registered filled state")
            })
    }
}
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FreezeFluids;
pub struct FluidPlugin;
impl Plugin for FluidPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FluidRegistrations>()
            .add_systems(PreStartup, freeze.after(FreezeContent).in_set(FreezeFluids));
    }
}
fn freeze(
    mut commands: Commands,
    content: Res<RegisteredContent>,
    mut registrations: ResMut<FluidRegistrations>,
) {
    let profiles = registrations.0.take().expect("Fluids already frozen");
    for (index, profile) in profiles.iter().enumerate() {
        assert!(profile.delay > 0 && profile.drop_off > 0);
        assert!(
            (2..=256).contains(&profile.standing.len())
                && profile.standing.len() == profile.falling.len()
        );
        assert!(profile.rising_delay_multiplier > 0 && profile.name.contains(':'));
        if let Some(render) = profile.render {
            assert!(
                render.still.index() < content.resource_count()
                    && render.flowing.index() < content.resource_count()
            );
            assert!(render.emission <= 15);
        }
        assert!(profile.contact.damage.is_finite() && profile.contact.damage >= 0.0);
        assert!(
            profile
                .contact
                .damage_resource
                .is_none_or(|id| id.index() < content.resource_count())
        );
        assert!(
            profile
                .renewable_sources
                .is_none_or(|count| (1..=4).contains(&count))
        );
        let movement = profile.movement;
        assert!(
            [
                movement.acceleration,
                movement.horizontal_drag,
                movement.vertical_drag,
                movement.sprint_drag,
                movement.deep_vertical_drag,
                movement.gravity
            ]
            .into_iter()
            .all(f64::is_finite)
        );
        if let Some(fog) = profile.fog {
            assert!(fog.color.into_iter().chain(fog.ambient).all(f32::is_finite));
            assert!(
                fog.start.is_finite()
                    && fog.end.is_finite()
                    && fog.end > fog.start
                    && fog.brightness.is_finite()
            );
        }
        for states in [&profile.standing, &profile.falling] {
            for (amount, state) in states.iter().enumerate() {
                assert!(state.index() < content.state_count());
                if amount > 0 {
                    assert_eq!(
                        content.state(*state).fluid.id.index(),
                        index + 1,
                        "Fluid state belongs to another profile"
                    );
                }
            }
        }
        for reaction in &profile.reactions {
            assert!(reaction.directions & !63 == 0);
            match reaction.contact {
                ContactFilter::Fluid(id) => {
                    assert!(id != FluidId::NONE && id.index() <= profiles.len())
                }
                ContactFilter::Block(id) => assert!(id.index() < content.block_count()),
            }
            assert!(
                reaction
                    .support
                    .is_none_or(|id| id.index() < content.block_count())
            );
            assert!(
                reaction.source_product.index() < content.state_count()
                    && reaction.flowing_product.index() < content.state_count()
            );
        }
    }
    for raw in 0..content.state_count() {
        let state = StateId(raw as u32);
        let definition = content.state(state);
        let fluid = definition.fluid;
        if fluid.id != FluidId::NONE {
            assert!(fluid.id.index() <= profiles.len());
            assert!(
                fluid.amount > 0 && fluid.amount <= profiles[fluid.id.index() - 1].max_amount()
            );
        }
        for container in content.containers(state) {
            assert!(container.fluid.index() <= profiles.len());
        }
    }
    commands.insert_resource(FluidCatalog(Arc::new(FluidData {
        content: content.0.clone(),
        profiles: profiles.into(),
    })));
}
fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
#[derive(Clone, Copy, Default)]
struct Passage {
    states: u64,
    direction: u8,
    valid: bool,
    open: bool,
}
pub struct FluidScratch<T: VoxelCell> {
    pub edits: Vec<CheckedEdit<T>>,
    passages: [Passage; 512],
    rectangles: Vec<[f32; 4]>,
    axes: [Vec<f32>; 2],
    frontier: Vec<(IVec3, u8)>,
    visited: Vec<u64>,
}
impl<T: VoxelCell> Default for FluidScratch<T> {
    fn default() -> Self {
        Self {
            edits: Vec::with_capacity(6),
            passages: [Passage::default(); 512],
            rectangles: Vec::new(),
            axes: [Vec::new(), Vec::new()],
            frontier: Vec::with_capacity(81),
            visited: vec![0; 2],
        }
    }
}
impl<T: VoxelCell> FluidScratch<T> {
    // A fixed-size direct-mapped cache bounds memory regardless of registry size.
    // Shapes with offsets use the position-aware path and never enter this cache.
    fn passage<W: FluidWorld<Cell = T>>(
        &mut self,
        source: T,
        source_pos: IVec3,
        catalog: &FluidCatalog,
        world: &W,
        target: T,
        delta: IVec3,
    ) -> bool {
        if catalog.definition(source).full_collision || catalog.definition(target).full_collision {
            return false;
        }
        if catalog.shapes(source).is_empty() && catalog.shapes(target).is_empty() {
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
        let pair = u64::from(source.id()) | (u64::from(target.id()) << 32);
        let direction = axis as u8 * 2 + u8::from(positive);
        let slot = (mix(pair ^ u64::from(direction)) >> 55) as usize;
        // Position-dependent geometry cannot use a state-only cache key.
        let source_offset = world.shape_offset(source, source_pos);
        let target_offset = world.shape_offset(target, source_pos + delta);
        let cacheable = source_offset.is_none() && target_offset.is_none();
        if cacheable
            && self.passages[slot].valid
            && self.passages[slot].states == pair
            && self.passages[slot].direction == direction
        {
            return self.passages[slot].open;
        }
        self.rectangles.clear();
        self.axes.iter_mut().for_each(|axis| {
            axis.clear();
            axis.extend([0.0, 1.0]);
        });
        let a = (axis + 1) % 3;
        let b = (axis + 2) % 3;
        for (state, offset, face) in [
            (source, source_offset.unwrap_or(Vec3::ZERO), positive),
            (target, target_offset.unwrap_or(Vec3::ZERO), !positive),
        ] {
            for &bounds in catalog.shapes(state) {
                let bounds = bounds.translated(offset);
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
                states: pair,
                direction,
                valid: true,
                open,
            };
        }
        open
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FlowControl {
    Continue,
    Stop,
}
/// Optional behavior hooks may sample and stage arbitrary checked edits. No heap
/// allocation or virtual call is added to the standard propagation path.
pub trait FluidAccess {
    fn catalog(&self) -> &FluidCatalog;
    fn origin(&self) -> IVec3;
    fn read(&self, position: IVec3) -> Result<StateId, IVec2>;
    fn write(&mut self, position: IVec3, state: StateId) -> Result<(), IVec2>;
}
/// An optional hook owns its typed configuration once per profile. It is shared
/// through the catalog, with no allocation or reference-count update per step.
pub trait FluidBehavior: Send + Sync + 'static {
    fn before_step(&self, access: &mut dyn FluidAccess) -> Result<FlowControl, IVec2>;
}
impl<F> FluidBehavior for F
where
    F: Fn(&mut dyn FluidAccess) -> Result<FlowControl, IVec2> + Send + Sync + 'static,
{
    fn before_step(&self, access: &mut dyn FluidAccess) -> Result<FlowControl, IVec2> {
        self(access)
    }
}
pub trait FluidWorld {
    type Cell: VoxelCell;
    fn read(&self, position: IVec3) -> Result<Self::Cell, IVec2>;
    fn vertical_bounds(&self) -> std::ops::Range<i32>;
    fn writable(&self, position: IVec3) -> Result<(), IVec2>;
    /// Some(offset) marks position-dependent collision geometry, including a zero
    /// offset at this position. Those states bypass the state-pair passage cache.
    fn shape_offset(&self, _state: Self::Cell, _position: IVec3) -> Option<Vec3> {
        None
    }
}
struct FlowContext<'a, W: FluidWorld> {
    world: &'a W,
    catalog: &'a FluidCatalog,
}
impl<W: FluidWorld> FlowContext<'_, W> {
    fn read(&self, position: IVec3, scratch: &FluidScratch<W::Cell>) -> Result<W::Cell, IVec2> {
        if !self.world.vertical_bounds().contains(&position.y) {
            return Ok(W::Cell::air());
        }
        if let Some(edit) = scratch.edits.iter().find(|edit| edit.position == position) {
            return Ok(edit.state);
        }
        self.world.read(position)
    }
    fn write(
        &self,
        position: IVec3,
        state: W::Cell,
        scratch: &mut FluidScratch<W::Cell>,
    ) -> Result<(), IVec2> {
        if !self.world.vertical_bounds().contains(&position.y) {
            return Ok(());
        }
        self.world.writable(position)?;
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
        family: FluidId,
        scratch: &mut FluidScratch<W::Cell>,
    ) -> Result<W::Cell, IVec2> {
        let state = self.read(position, scratch)?;
        let (mut highest, mut sources) = (0, 0);
        for delta in SIDES {
            let neighbor = self.read(position + delta, scratch)?;
            let fluid = self.catalog.info(neighbor);
            if fluid.family == family
                && scratch.passage(state, position, self.catalog, self.world, neighbor, delta)
            {
                highest = highest.max(fluid.amount);
                sources += u8::from(fluid.source);
            }
        }
        if self
            .catalog
            .profile(family)
            .renewable_sources
            .is_some_and(|required| sources >= required)
        {
            let below = self.read(position - IVec3::Y, scratch)?;
            let fluid = self.catalog.info(below);
            if self.catalog.definition(below).solid || (fluid.family == family && fluid.source) {
                return Ok(self.catalog.liquid(
                    family,
                    self.catalog.profile(family).max_amount(),
                    false,
                ));
            }
        }
        let above = self.read(position + IVec3::Y, scratch)?;
        if self.catalog.info(above).family == family
            && scratch.passage(state, position, self.catalog, self.world, above, IVec3::Y)
        {
            return Ok(self.catalog.liquid(
                family,
                self.catalog.profile(family).max_amount(),
                true,
            ));
        }
        Ok(self.catalog.liquid(
            family,
            highest.saturating_sub(self.catalog.profile(family).drop_off),
            false,
        ))
    }
    fn hole(
        &self,
        position: IVec3,
        family: FluidId,
        scratch: &mut FluidScratch<W::Cell>,
    ) -> Result<bool, IVec2> {
        if position.y <= self.world.vertical_bounds().start {
            return Ok(false);
        }
        let state = self.read(position, scratch)?;
        let below = self.read(position - IVec3::Y, scratch)?;
        Ok(scratch.passage(
            state,
            position,
            self.catalog,
            self.world,
            below,
            IVec3::NEG_Y,
        ) && (self.catalog.info(below).family == family || self.catalog.holds(below, family)))
    }
    fn slope(
        &self,
        position: IVec3,
        from: IVec3,
        family: FluidId,
        scratch: &mut FluidScratch<W::Cell>,
    ) -> Result<u8, IVec2> {
        if self.hole(position, family, scratch)? {
            return Ok(0);
        }
        scratch.frontier.clear();
        scratch.frontier.push((position, 0));
        let radius = i32::from(self.catalog.profile(family).slope_limit);
        let width = radius * 2 + 1;
        scratch.visited.resize((width * width) as usize / 64 + 1, 0);
        scratch.visited.fill(0);
        let visit_bit = |p: IVec3| {
            let offset = p - position;
            ((offset.x + radius) + (offset.z + radius) * width) as usize
        };
        // The incoming cell may lie outside a zero-radius search; it is never explored.
        if radius > 0 {
            let bit = visit_bit(from);
            scratch.visited[bit / 64] |= 1 << (bit % 64);
        }
        let bit = visit_bit(position);
        scratch.visited[bit / 64] |= 1 << (bit % 64);
        let mut cursor = 0;
        while cursor < scratch.frontier.len() {
            let (pos, distance) = scratch.frontier[cursor];
            cursor += 1;
            if distance >= self.catalog.profile(family).slope_limit {
                continue;
            }
            let state = self.read(pos, scratch)?;
            for delta in SIDES {
                let next = pos + delta;
                let bit = visit_bit(next);
                if scratch.visited[bit / 64] & (1 << (bit % 64)) != 0 {
                    continue;
                }
                let target = self.read(next, scratch)?;
                let fluid = self.catalog.info(target);
                if (fluid.family == family && fluid.source)
                    || !self.catalog.holds(target, family)
                    || !scratch.passage(state, pos, self.catalog, self.world, target, delta)
                {
                    continue;
                }
                scratch.visited[bit / 64] |= 1 << (bit % 64);
                if self.hole(next, family, scratch)? {
                    return Ok(distance + 1);
                }
                scratch.frontier.push((next, distance + 1));
            }
        }
        Ok(u8::MAX)
    }
}

fn flow<W: FluidWorld>(
    context: &FlowContext<W>,
    position: IVec3,
    mut state: W::Cell,
    scratch: &mut FluidScratch<W::Cell>,
) -> Result<(), IVec2> {
    let mut fluid = context.catalog.info(state);
    if fluid.family == FluidId::NONE {
        return Ok(());
    }
    if context.react(position, state, scratch)? {
        return Ok(());
    }
    if !fluid.source {
        let next = context.new_liquid(position, fluid.family, scratch)?;
        if state != next {
            context.write(position, next, scratch)?;
            state = next;
            fluid = context.catalog.info(state);
        }
        if fluid.family == FluidId::NONE {
            return Ok(());
        }
    }
    let mut down = false;
    if position.y > context.world.vertical_bounds().start {
        let below_pos = position - IVec3::Y;
        let below = context.read(below_pos, scratch)?;
        let target = context.catalog.info(below);
        if scratch.passage(
            state,
            position,
            context.catalog,
            context.world,
            below,
            IVec3::NEG_Y,
        ) {
            if target.family == FluidId::NONE && context.catalog.holds(below, fluid.family) {
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
    if !fluid.falling && fluid.amount <= context.catalog.profile(fluid.family).drop_off {
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
            || !scratch.passage(
                state,
                position,
                context.catalog,
                context.world,
                target,
                delta,
            )
        {
            continue;
        }
        let next = context.new_liquid(target_pos, fluid.family, scratch)?;
        if context.catalog.info(next).family == FluidId::NONE {
            continue;
        }
        let distance = context.slope(target_pos, position, fluid.family, scratch)?;
        if distance < lowest {
            choices.fill(None);
            lowest = distance;
        }
        if distance <= lowest {
            if existing.family == FluidId::NONE {
                choices[i] = Some(context.catalog.destination(target, next));
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

impl<W: FluidWorld> FlowContext<'_, W> {
    fn react(
        &self,
        position: IVec3,
        state: W::Cell,
        scratch: &mut FluidScratch<W::Cell>,
    ) -> Result<bool, IVec2> {
        let fluid = self.catalog.info(state);
        let mut changed = false;
        for rule in &self.catalog.profile(fluid.family).reactions {
            if let Some(support) = rule.support {
                let below = self.read(position - IVec3::Y, scratch)?;
                if self.catalog.definition(below).block != support {
                    continue;
                }
            }
            for (i, delta) in SIDES
                .into_iter()
                .chain([IVec3::Y, IVec3::NEG_Y])
                .enumerate()
            {
                if rule.directions & (1 << i) == 0 {
                    continue;
                }
                let neighbor_position = position + delta;
                let neighbor = self.read(neighbor_position, scratch)?;
                let info = self.catalog.info(neighbor);
                let matches = match rule.contact {
                    ContactFilter::Fluid(id) => info.family == id,
                    ContactFilter::Block(id) => self.catalog.definition(neighbor).block == id,
                };
                if !matches
                    || info.amount < rule.minimum_amount
                    || (rule.free_fluid_only && info.container)
                {
                    continue;
                }
                let (target, target_state, source) = match rule.target {
                    ReactionTarget::Origin => (position, state, fluid.source),
                    ReactionTarget::Neighbor => (neighbor_position, neighbor, info.source),
                };
                let product = if source {
                    rule.source_product
                } else {
                    rule.flowing_product
                };
                let product = W::Cell::from_id(product.0, self.catalog.content())
                    .expect("Registered reaction product");
                if target_state != product {
                    self.write(target, product, scratch)?;
                    changed = true;
                }
                if matches!(rule.target, ReactionTarget::Origin) {
                    return Ok(true);
                }
            }
        }
        Ok(changed)
    }
}
/// Prepare one atomic batch. Boundary failures leave terrain untouched; callers
/// commit only Ok batches and retain their scheduling/budget policy.
pub fn step<W: FluidWorld>(
    catalog: &FluidCatalog,
    world: &W,
    position: IVec3,
    scratch: &mut FluidScratch<W::Cell>,
) -> Result<(), IVec2> {
    scratch.edits.clear();
    if !world.vertical_bounds().contains(&position.y) {
        return Ok(());
    }
    let state = world.read(position)?;
    let fluid = catalog.info(state);
    if fluid.family == FluidId::NONE {
        return Ok(());
    }
    let context = FlowContext { world, catalog };
    let result = (|| {
        if let Some(hook) = &catalog.profile(fluid.family).before_step {
            let mut access = HookAccess {
                context: &context,
                scratch,
                position,
            };
            if hook.before_step(&mut access)? == FlowControl::Stop {
                return Ok(());
            }
        }
        flow(&context, position, state, scratch)
    })();
    if result.is_err() {
        scratch.edits.clear();
    } else {
        scratch.edits.retain(|edit| edit.expected != edit.state);
    }
    result
}

struct HookAccess<'a, 'b, W: FluidWorld> {
    context: &'a FlowContext<'b, W>,
    scratch: &'a mut FluidScratch<W::Cell>,
    position: IVec3,
}
impl<W: FluidWorld> FluidAccess for HookAccess<'_, '_, W> {
    fn catalog(&self) -> &FluidCatalog {
        self.context.catalog
    }
    fn origin(&self) -> IVec3 {
        self.position
    }
    fn read(&self, position: IVec3) -> Result<StateId, IVec2> {
        self.context
            .read(position, self.scratch)
            .map(|s| StateId(s.id()))
    }
    fn write(&mut self, position: IVec3, state: StateId) -> Result<(), IVec2> {
        self.context.write(
            position,
            W::Cell::from_id(state.0, self.context.catalog.content())
                .expect("Registered hook state"),
            self.scratch,
        )
    }
}

pub mod surface;
