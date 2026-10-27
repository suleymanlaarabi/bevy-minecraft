//! Frozen content tables. Names and variable-size schemas exist only at loading boundaries.
use crate::{BlockId, FluidId, ResourceId, ShapeId, StateId, shape::BlockBox};
use std::{collections::HashMap, ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PropertyId(pub u16);
#[derive(Clone, Debug)]
pub struct PropertySpec {
    pub name: String,
    pub values: Vec<String>,
    pub default: u16,
}
#[derive(Clone, Debug)]
pub struct PropertyDefinition {
    pub name: String,
    pub values: Box<[String]>,
    pub stride: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ContainerRange {
    pub start: u32,
    pub end: u32,
}
impl ContainerRange {
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}
#[derive(Clone, Copy, Debug)]
pub struct FluidContainer {
    pub fluid: FluidId,
    pub filled: StateId,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FluidState {
    pub id: FluidId,
    pub amount: u8,
    pub source: bool,
    pub falling: bool,
    pub embedded: bool,
}

/// Common physical metadata, not a closed list of gameplay behaviors.
/// Additional capabilities belong in plugin-owned tables indexed by StateId.
#[derive(Clone, Copy, Debug)]
pub struct StateDefinition {
    pub block: BlockId,
    pub fluid: FluidState,
    pub washable: bool,
    pub containers: ContainerRange,
    pub collision: ShapeId,
    pub selection: ShapeId,
    pub opacity: u8,
    pub luminance: u8,
    pub hardness: f32,
    pub air: bool,
    pub replaceable: bool,
    pub solid: bool,
    pub full_cube: bool,
    pub full_collision: bool,
    pub occludes: bool,
    pub solid_render: bool,
    pub tool_required: bool,
    pub solid_faces: u8,
    pub center_faces: u8,
}
impl Default for StateDefinition {
    fn default() -> Self {
        Self {
            block: BlockId(0),
            fluid: FluidState::default(),
            washable: false,
            containers: ContainerRange::default(),
            collision: ShapeId(0),
            selection: ShapeId(0),
            opacity: 0,
            luminance: 0,
            hardness: 1.0,
            air: false,
            replaceable: false,
            solid: false,
            full_cube: false,
            full_collision: false,
            occludes: false,
            solid_render: false,
            tool_required: false,
            solid_faces: 0,
            center_faces: 0,
        }
    }
}
impl StateDefinition {
    pub fn is_air(&self) -> bool {
        self.air
    }
    pub fn replaceable(&self) -> bool {
        self.replaceable
    }
    pub fn is_solid(&self) -> bool {
        self.solid
    }
    pub fn is_full_cube(&self) -> bool {
        self.full_cube
    }
    pub fn can_occlude(&self) -> bool {
        self.occludes
    }
    pub fn is_solid_render(&self) -> bool {
        self.solid_render
    }
    pub fn tool_required(&self) -> bool {
        self.tool_required
    }
}
#[derive(Clone, Debug)]
pub struct BlockDefinition {
    pub name: String,
    pub states: Range<u32>,
    pub default_state: StateId,
    pub properties: Box<[PropertyDefinition]>,
    pub resources: Box<[ResourceId]>,
    pub slipperiness: f32,
    pub velocity_multiplier: f32,
    pub jump_velocity_multiplier: f32,
}
#[derive(Clone, Debug)]
pub struct BlockSpec {
    pub name: String,
    pub properties: Vec<PropertySpec>,
    pub resources: Vec<ResourceId>,
    /// A shared initial profile; plugins can override individual registered states.
    pub state: StateDefinition,
    pub slipperiness: f32,
    pub velocity_multiplier: f32,
    pub jump_velocity_multiplier: f32,
}
impl BlockSpec {
    pub fn new(name: impl Into<String>, state: StateDefinition) -> Self {
        Self {
            name: name.into(),
            properties: Vec::new(),
            resources: Vec::new(),
            state,
            slipperiness: 0.6,
            velocity_multiplier: 1.0,
            jump_velocity_multiplier: 1.0,
        }
    }
}
/// Optional cuboid-surface capability. Arbitrary model/UI/audio resources remain
/// plugin-owned; their presence never implicitly opts a block into this renderer.
#[derive(Clone, Copy, Debug)]
pub struct BlockVisual {
    pub faces: [ResourceId; 6],
    pub translucent: bool,
}
struct ContentData {
    blocks: Box<[BlockDefinition]>,
    states: Box<[StateDefinition]>,
    shapes: Box<[Box<[BlockBox]>]>,
    resources: Box<[String]>,
    containers: Box<[FluidContainer]>,
    visuals: Option<Box<[Option<BlockVisual>]>>,
    // Cold import/UI name resolution only. Runtime dispatch directly indexes the tables above.
    name_order: Box<[BlockId]>,
}
#[derive(Clone)]
pub struct ContentCatalog(Arc<ContentData>);
impl ContentCatalog {
    pub fn block(&self, id: BlockId) -> &BlockDefinition {
        &self.0.blocks[id.index()]
    }
    pub fn state(&self, id: StateId) -> &StateDefinition {
        &self.0.states[id.index()]
    }
    pub fn shapes(&self, id: ShapeId) -> &[BlockBox] {
        &self.0.shapes[id.index()]
    }
    pub fn resource(&self, id: ResourceId) -> &str {
        &self.0.resources[id.index()]
    }
    pub fn containers(&self, state: StateId) -> &[FluidContainer] {
        let range = self.state(state).containers;
        &self.0.containers[range.start as usize..range.end as usize]
    }
    pub fn container(&self, state: StateId, fluid: FluidId) -> Option<FluidContainer> {
        let rules = self.containers(state);
        rules
            .binary_search_by_key(&fluid.0, |rule| rule.fluid.0)
            .ok()
            .map(|i| rules[i])
    }
    pub fn resolve(&self, name: &str) -> Option<BlockId> {
        self.0
            .name_order
            .binary_search_by(|id| self.block(*id).name.as_str().cmp(name))
            .ok()
            .map(|i| self.0.name_order[i])
    }
    pub fn visual(&self, block: BlockId) -> Option<BlockVisual> {
        self.0
            .visuals
            .as_ref()
            .and_then(|visuals| visuals[block.index()])
    }
    pub fn resource_count(&self) -> usize {
        self.0.resources.len()
    }
    pub fn block_count(&self) -> usize {
        self.0.blocks.len()
    }
    pub fn state_count(&self) -> usize {
        self.0.states.len()
    }
    pub fn property(&self, block: BlockId, name: &str) -> Option<PropertyId> {
        self.block(block)
            .properties
            .iter()
            .position(|p| p.name == name)
            .map(|i| PropertyId(u16::try_from(i).expect("Property count validated")))
    }
    pub fn value(&self, state: StateId, property: PropertyId) -> u16 {
        let block = self.block(self.state(state).block);
        let spec = &block.properties[usize::from(property.0)];
        ((state.0 - block.states.start) / spec.stride % spec.values.len() as u32) as u16
    }
    pub fn with(&self, state: StateId, property: PropertyId, value: u16) -> Option<StateId> {
        let block = self.block(self.state(state).block);
        let spec = block.properties.get(usize::from(property.0))?;
        if usize::from(value) >= spec.values.len() {
            return None;
        }
        let old = self.value(state, property);
        Some(StateId(
            state.0 - u32::from(old) * spec.stride + u32::from(value) * spec.stride,
        ))
    }
}
/// Consumed at initialization; runtime state tables have no mutable registration path.
pub struct ContentBuilder {
    pub blocks: Vec<BlockDefinition>,
    pub states: Vec<StateDefinition>,
    shapes: Vec<Box<[BlockBox]>>,
    resources: Vec<String>,
    containers: Vec<(StateId, FluidContainer)>,
    visuals: Vec<(BlockId, BlockVisual)>,
    names: HashMap<String, BlockId>,
}
impl Default for ContentBuilder {
    fn default() -> Self {
        Self::new()
    }
}
impl ContentBuilder {
    pub fn new() -> Self {
        Self {
            shapes: vec![Box::new([])],
            blocks: Vec::new(),
            states: Vec::new(),
            resources: Vec::new(),
            containers: Vec::new(),
            visuals: Vec::new(),
            names: HashMap::new(),
        }
    }
    pub fn resolve(&self, name: &str) -> Option<BlockId> {
        self.names.get(name).copied()
    }
    pub fn shape(&mut self, boxes: impl Into<Box<[BlockBox]>>) -> ShapeId {
        let id = ShapeId(u32::try_from(self.shapes.len()).expect("Too many shapes"));
        self.shapes.push(boxes.into());
        id
    }
    pub fn resource(&mut self, path: impl Into<String>) -> ResourceId {
        let path = path.into();
        // Registration is cold and resource lists are small; no retained duplicate name index.
        if let Some(i) = self.resources.iter().position(|p| *p == path) {
            return ResourceId(i as u32);
        }
        let id = ResourceId(u32::try_from(self.resources.len()).expect("Too many resources"));
        self.resources.push(path);
        id
    }
    pub fn set_visual(&mut self, block: BlockId, visual: BlockVisual) {
        assert!(
            !self.visuals.iter().any(|(id, _)| *id == block),
            "Duplicate block visual"
        );
        self.visuals.push((block, visual));
    }
    pub fn accept_fluid(&mut self, state: StateId, fluid: FluidId, filled: StateId) {
        self.containers
            .push((state, FluidContainer { fluid, filled }));
    }
    pub fn register(&mut self, spec: BlockSpec) -> BlockId {
        assert!(
            spec.properties.len() <= u16::MAX as usize,
            "Too many properties"
        );
        let block = BlockId(u32::try_from(self.blocks.len()).expect("Too many blocks"));
        let start = u32::try_from(self.states.len()).expect("Too many states");
        let mut count = 1u32;
        let mut default_offset = 0;
        let mut properties = Vec::with_capacity(spec.properties.len());
        for property in spec.properties.into_iter().rev() {
            assert!(!property.values.is_empty() && property.values.len() <= u16::MAX as usize);
            assert!(usize::from(property.default) < property.values.len());
            assert!(
                !properties
                    .iter()
                    .any(|p: &PropertyDefinition| p.name == property.name),
                "Duplicate property"
            );
            for (i, value) in property.values.iter().enumerate() {
                assert!(
                    !property.values[..i].contains(value),
                    "Duplicate property value"
                );
            }
            default_offset += u32::from(property.default) * count;
            properties.push(PropertyDefinition {
                name: property.name,
                values: property.values.into(),
                stride: count,
            });
            count = count
                .checked_mul(properties.last().unwrap().values.len() as u32)
                .expect("State domain overflow");
        }
        properties.reverse();
        let end = start.checked_add(count).expect("Too many states");
        self.states.resize(
            end as usize,
            StateDefinition {
                block,
                ..spec.state
            },
        );
        self.import_block(BlockDefinition {
            name: spec.name,
            states: start..end,
            default_state: StateId(start + default_offset),
            properties: properties.into(),
            resources: spec.resources.into(),
            slipperiness: spec.slipperiness,
            velocity_multiplier: spec.velocity_multiplier,
            jump_velocity_multiplier: spec.jump_velocity_multiplier,
        });
        block
    }
    /// Import providers retain their existing dense numeric identities without translation tables.
    pub fn import_block(&mut self, block: BlockDefinition) {
        assert!(
            block.name.contains(':') && !self.names.contains_key(&block.name),
            "Duplicate or unqualified block name"
        );
        assert!(block.states.contains(&block.default_state.0));
        let id = BlockId(u32::try_from(self.blocks.len()).expect("Too many blocks"));
        self.names.insert(block.name.clone(), id);
        self.blocks.push(block);
    }
    pub fn freeze(mut self) -> ContentCatalog {
        self.containers
            .sort_unstable_by_key(|(state, rule)| (state.0, rule.fluid.0));
        for pair in self.containers.windows(2) {
            assert!(
                pair[0].0 != pair[1].0 || pair[0].1.fluid != pair[1].1.fluid,
                "Duplicate fluid container rule"
            );
        }
        for (index, (state, rule)) in self.containers.iter().enumerate() {
            assert!(rule.filled.index() < self.states.len() && rule.fluid != FluidId::NONE);
            let range = &mut self.states[state.index()].containers;
            if range.is_empty() {
                range.start = u32::try_from(index).expect("Too many container rules");
            }
            range.end = u32::try_from(index + 1).expect("Too many container rules");
        }

        for (i, block) in self.blocks.iter().enumerate() {
            assert!(block.states.end as usize <= self.states.len());
            for state in &self.states[block.states.start as usize..block.states.end as usize] {
                assert_eq!(state.block.index(), i, "State belongs to another block");
            }
            assert!(
                block
                    .resources
                    .iter()
                    .all(|id| id.index() < self.resources.len())
            );
        }
        for state in &self.states {
            assert!(
                state.collision.index() < self.shapes.len()
                    && state.selection.index() < self.shapes.len()
            );
            assert!(state.opacity <= 15 && state.luminance <= 15 && state.hardness.is_finite());
        }
        let visuals = if self.visuals.is_empty() {
            None
        } else {
            let mut entries = vec![None; self.blocks.len()];
            for (block, visual) in self.visuals {
                assert!(
                    visual
                        .faces
                        .iter()
                        .all(|id| id.index() < self.resources.len())
                );
                entries[block.index()] = Some(visual);
            }
            Some(entries.into_boxed_slice())
        };
        let mut name_order: Vec<_> = (0..self.blocks.len()).map(|i| BlockId(i as u32)).collect();
        name_order.sort_unstable_by(|a, b| {
            self.blocks[a.index()]
                .name
                .cmp(&self.blocks[b.index()].name)
        });
        ContentCatalog(Arc::new(ContentData {
            blocks: self.blocks.into(),
            states: self.states.into(),
            shapes: self.shapes.into(),
            resources: self.resources.into(),
            containers: self.containers.into_iter().map(|(_, rule)| rule).collect(),
            name_order: name_order.into_boxed_slice(),
            visuals,
        }))
    }
}
