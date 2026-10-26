//! Allocation-free access to native mixed-radix block properties.
use crate::{Block, BlockStateId, Property, PropertyKey};

#[path = "generated/property_domains.rs"]
mod domains;

pub(super) struct PropertySpec {
    values: &'static [Property],
    stride: u16,
}

impl PropertySpec {
    fn value(&self, index: u16) -> Property {
        self.values[(index / self.stride) as usize % self.values.len()]
    }
}

/// An unsupported property or value; no partial update is published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidBlockProperty(pub Property);

impl std::fmt::Display for InvalidBlockProperty {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Invalid block property: {:?}", self.0)
    }
}
impl std::error::Error for InvalidBlockProperty {}

impl BlockStateId {
    pub fn properties(self) -> impl ExactSizeIterator<Item = Property> + Clone {
        let block = self.to_block();
        let index = self.as_u16() - block.states[0].id.as_u16();
        domains::specs(block.id)
            .iter()
            .map(move |spec| spec.value(index))
    }

    pub fn property(self, key: PropertyKey) -> Option<Property> {
        let block = self.to_block();
        let spec = domains::specs(block.id)
            .iter()
            .find(|spec| spec.values[0].key() == key)?;
        Some(spec.value(self.as_u16() - block.states[0].id.as_u16()))
    }

    pub fn supports(self, property: Property) -> bool {
        domains::specs(self.to_block_id()).iter().any(|spec| {
            spec.values
                .iter()
                .any(|&value| value.matches_property(property))
        })
    }

    pub fn with_properties(self, edits: &[Property]) -> Result<Self, InvalidBlockProperty> {
        let block = self.to_block();
        let specs = domains::specs(block.id);
        let mut index = self.as_u16() - block.states[0].id.as_u16();
        for &edit in edits {
            let spec = specs
                .iter()
                .find(|spec| spec.values[0].key() == edit.key())
                .ok_or(InvalidBlockProperty(edit))?;
            let value = spec
                .values
                .iter()
                .position(|&value| value.matches_property(edit))
                .ok_or(InvalidBlockProperty(edit))? as u16;
            let old = index / spec.stride % spec.values.len() as u16;
            index = index - old * spec.stride + value * spec.stride;
        }
        Ok(block.states[index as usize].id)
    }
}

impl Block {
    pub fn state_from_properties(
        &'static self,
        properties: &[Property],
    ) -> Option<&'static crate::BlockState> {
        if properties.len() != domains::specs(self.id).len() {
            return None;
        }
        let state = self.default_state.id.with_properties(properties).ok()?;
        state
            .properties()
            .all(|property| {
                properties
                    .iter()
                    .any(|&requested| property.matches_property(requested))
            })
            .then(|| state.to_state())
    }

    /// Copy only properties supported by the destination, preserving its other defaults.
    pub fn copy_properties(&self, source: BlockStateId) -> BlockStateId {
        let mut state = self.default_state.id;
        for property in source
            .properties()
            .filter(|&property| state_supports(self, property))
        {
            state = state
                .with_properties(&[property])
                .expect("Supported destination property");
        }
        state
    }
}

fn state_supports(block: &Block, property: Property) -> bool {
    block.default_state.id.supports(property)
}

const _: () = assert!(std::mem::size_of::<BlockStateId>() == 2);

/// Stack storage for transformations which need a complete mutable property set.
/// The generator checks that every native family fits this bound.
#[derive(Clone, Debug)]
pub struct PropertyValues {
    values: [Property; 16],
    len: usize,
}
impl FromIterator<Property> for PropertyValues {
    fn from_iter<T: IntoIterator<Item = Property>>(iter: T) -> Self {
        let mut values = Self {
            values: [Property::Waterlogged(false); 16],
            len: 0,
        };
        for property in iter {
            values.values[values.len] = property;
            values.len += 1;
        }
        values
    }
}
impl std::ops::Deref for PropertyValues {
    type Target = [Property];
    fn deref(&self) -> &[Property] {
        &self.values[..self.len]
    }
}
