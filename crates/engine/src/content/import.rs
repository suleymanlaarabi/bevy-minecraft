//! Declarative input is resolved once into the same tables as Rust registrations.
use crate::fluids::{FluidProfile, FluidRegistrations, FluidRender};
use bevy_math::Vec3;
use hollow_core::{
    content::{BlockSpec, BlockVisual, ContentBuilder, PropertySpec, StateDefinition},
    shape::BlockBox,
};
use serde::Deserialize;
use std::io;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentManifest {
    #[serde(default)]
    pub blocks: Vec<BlockDeclaration>,
    #[serde(default)]
    pub fluids: Vec<FluidDeclaration>,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub containers: Vec<ContainerDeclaration>,
    #[serde(default)]
    pub states: Vec<StateOverride>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateReference {
    pub block: String,
    #[serde(default)]
    pub properties: Vec<(String, String)>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerDeclaration {
    pub empty: StateReference,
    pub filled: StateReference,
    pub fluid: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateOverride {
    pub state: StateReference,
    pub emission: Option<u8>,
    pub opacity: Option<u8>,
    pub washable: Option<bool>,
}
impl StateReference {
    fn resolve(&self, source: &str, content: &ContentBuilder) -> io::Result<hollow_core::StateId> {
        let block = content
            .resolve(&self.block)
            .ok_or_else(|| invalid(source, &self.block, "Unknown block"))?;
        let definition = &content.blocks[block.index()];
        let mut state = definition.default_state.0;
        for (index, (name, value)) in self.properties.iter().enumerate() {
            if self.properties[..index]
                .iter()
                .any(|(previous, _)| previous == name)
            {
                return Err(invalid(source, name, "Duplicate state property"));
            }
            let property = definition
                .properties
                .iter()
                .find(|property| property.name == *name)
                .ok_or_else(|| invalid(source, name, "Unknown state property"))?;
            let value = property
                .values
                .iter()
                .position(|candidate| candidate == value)
                .ok_or_else(|| invalid(source, name, "Unknown property value"))?
                as u32;
            let old = ((state - definition.states.start) / property.stride)
                % property.values.len() as u32;
            state = state - old * property.stride + value * property.stride;
        }
        Ok(hollow_core::StateId(state))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyDeclaration {
    pub name: String,
    pub values: Vec<String>,
    pub default: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cuboid {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BlockDeclaration {
    pub name: String,
    pub properties: Vec<PropertyDeclaration>,
    pub shape: Vec<Cuboid>,
    pub textures: Option<[String; 6]>,
    pub solid: bool,
    pub opacity: u8,
    pub emission: u8,
    pub replaceable: bool,
    pub translucent: bool,
    pub air: bool,
}
impl Default for BlockDeclaration {
    fn default() -> Self {
        Self {
            name: String::new(),
            properties: Vec::new(),
            shape: Vec::new(),
            textures: None,
            solid: false,
            opacity: 0,
            emission: 0,
            replaceable: false,
            translucent: false,
            air: false,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FluidDeclaration {
    pub name: String,
    pub block: BlockDeclaration,
    pub maximum: u8,
    pub delay: u64,
    pub still: String,
    pub flowing: String,
    #[serde(default)]
    pub renewable_sources: Option<u8>,
}
fn invalid(source: &str, element: &str, reason: &str) -> io::Error {
    io::Error::other(format!("{source}: {element}: {reason}"))
}
impl ContentManifest {
    pub fn parse(source: &str, bytes: &[u8]) -> io::Result<Self> {
        ron::de::from_bytes(bytes).map_err(|error| invalid(source, "manifest", &error.to_string()))
    }
    /// Must run before catalog freezing. Errors abort application initialization.
    pub fn register(
        self,
        source: &str,
        content: &mut ContentBuilder,
        fluids: &mut FluidRegistrations,
    ) -> io::Result<()> {
        for resource in self.resources {
            content.resource(resource);
        }
        for block in self.blocks {
            let visual = block.textures.as_ref().map(|faces| BlockVisual {
                faces: std::array::from_fn(|index| content.resource(faces[index].clone())),
                translucent: block.translucent,
            });
            let spec = block.compile(source, content)?;
            let block = content.register(spec);
            if let Some(visual) = visual {
                content.set_visual(block, visual);
            }
        }
        for fluid in self.fluids {
            if fluids.resolve(&fluid.name).is_some()
                || fluid.maximum == 0
                || fluid.delay == 0
                || !fluid.block.properties.is_empty()
            {
                return Err(invalid(
                    source,
                    &fluid.name,
                    "Invalid or duplicate simple fluid",
                ));
            }
            let mut block = fluid.block.compile(source, content)?;
            block.state.replaceable = true;
            block.state.washable = true;
            let mut profile = FluidProfile::new(fluid.name);
            profile.delay = fluid.delay;
            profile.renewable_sources = fluid.renewable_sources;
            profile.render = Some(FluidRender {
                still: content.resource(fluid.still),
                flowing: content.resource(fluid.flowing),
                translucent: true,
                emission: block.state.luminance,
                effect: 0,
                ocean_surface: false,
            });
            fluids.register_simple(content, block, profile, fluid.maximum);
        }
        for patch in self.states {
            let state = patch.state.resolve(source, content)?;
            let definition = &mut content.states[state.index()];
            if patch.emission.is_some_and(|value| value > 15)
                || patch.opacity.is_some_and(|value| value > 15)
            {
                return Err(invalid(
                    source,
                    &patch.state.block,
                    "Light must be in 0..=15",
                ));
            }
            if let Some(value) = patch.emission {
                definition.luminance = value;
            }
            if let Some(value) = patch.opacity {
                definition.opacity = value;
            }
            if let Some(value) = patch.washable {
                definition.washable = value;
            }
        }
        for container in self.containers {
            let empty = container.empty.resolve(source, content)?;
            let filled = container.filled.resolve(source, content)?;
            let fluid = fluids
                .resolve(&container.fluid)
                .ok_or_else(|| invalid(source, &container.fluid, "Unknown container fluid"))?;
            if content.states[filled.index()].fluid.id != hollow_core::FluidId::NONE {
                return Err(invalid(
                    source,
                    &container.filled.block,
                    "Container state already contains fluid",
                ));
            }
            let maximum = fluids.profile_mut(fluid).max_amount();
            content.states[filled.index()].fluid = hollow_core::content::FluidState {
                id: fluid,
                amount: maximum,
                source: true,
                falling: false,
                embedded: true,
            };
            content.accept_fluid(empty, fluid, filled);
        }
        if content.states.first().is_some_and(|state| !state.air) {
            return Err(invalid(source, "state 0", "The first state must be air"));
        }
        Ok(())
    }
}
impl BlockDeclaration {
    fn compile(self, source: &str, content: &mut ContentBuilder) -> io::Result<BlockSpec> {
        if self
            .name
            .split_once(':')
            .is_none_or(|(namespace, name)| namespace.is_empty() || name.is_empty())
            || content.resolve(&self.name).is_some()
            || self.opacity > 15
            || self.emission > 15
        {
            return Err(invalid(source, &self.name, "Invalid or duplicate block"));
        }
        if self.air && !content.states.is_empty() {
            return Err(invalid(
                source,
                &self.name,
                "Air must be the first registered state",
            ));
        }
        let mut boxes = Vec::with_capacity(self.shape.len());
        for cuboid in self.shape {
            let min = Vec3::from_array(cuboid.min);
            let max = Vec3::from_array(cuboid.max);
            if !min.is_finite()
                || !max.is_finite()
                || !min.cmpge(Vec3::ZERO).all()
                || !max.cmple(Vec3::ONE).all()
                || !min.cmplt(max).all()
            {
                return Err(invalid(source, &self.name, "Invalid collision cuboid"));
            }
            boxes.push(BlockBox { min, max });
        }
        let full_cube = boxes.len() == 1 && boxes[0].min == Vec3::ZERO && boxes[0].max == Vec3::ONE;
        let shape = content.shape(boxes);
        let mut spec = BlockSpec::new(
            self.name,
            StateDefinition {
                collision: if self.solid {
                    shape
                } else {
                    hollow_core::ShapeId(0)
                },
                selection: shape,
                full_cube,
                full_collision: full_cube && self.solid,
                solid: self.solid,
                occludes: self.opacity == 15,
                solid_render: self.opacity == 15,
                solid_faces: if full_cube && self.solid { 63 } else { 0 },
                center_faces: if full_cube && self.solid { 63 } else { 0 },
                opacity: self.opacity,
                luminance: self.emission,
                replaceable: self.replaceable,
                air: self.air,
                ..Default::default()
            },
        );
        let mut states = 1u32;
        for property in self.properties {
            if property.name.is_empty()
                || spec
                    .properties
                    .iter()
                    .any(|previous| previous.name == property.name)
                || property.values.is_empty()
                || property.values.len() > u16::MAX as usize
            {
                return Err(invalid(source, &spec.name, "Invalid property domain"));
            }
            let default = property
                .values
                .iter()
                .position(|value| *value == property.default)
                .ok_or_else(|| invalid(source, &property.name, "Unknown default value"))?;
            for (index, value) in property.values.iter().enumerate() {
                if property.values[..index].contains(value) {
                    return Err(invalid(source, &property.name, "Duplicate property value"));
                }
            }
            states = states
                .checked_mul(property.values.len() as u32)
                .ok_or_else(|| invalid(source, &spec.name, "State domain overflow"))?;
            spec.properties.push(PropertySpec {
                name: property.name,
                values: property.values,
                default: default as u16,
            });
        }
        if content
            .states
            .len()
            .checked_add(states as usize)
            .is_none_or(|count| count > u32::MAX as usize)
        {
            return Err(invalid(source, &spec.name, "State domain overflow"));
        }
        Ok(spec)
    }
}
