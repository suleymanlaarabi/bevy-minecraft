//! Text block states are validated and resolved once during deserialization.
use pumpkin_data::{Block, BlockState, BlockStateId, Property};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub struct BlockStateCodec(BlockStateId);

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TextBlockState {
    name: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    properties: HashMap<String, String>,
}

impl<'de> Deserialize<'de> for BlockStateCodec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = TextBlockState::deserialize(deserializer)?;
        let block =
            Block::from_registry_key(value.name.strip_prefix("minecraft:").unwrap_or(&value.name))
                .ok_or_else(|| {
                    serde::de::Error::custom(format!("Unknown block: {}", value.name))
                })?;
        let mut state = block.default_state.id;
        for (key, value) in value.properties {
            let property = Property::from_str_pair(&key, &value).ok_or_else(|| {
                serde::de::Error::custom(format!("Invalid block property: {key}={value}"))
            })?;
            state = state
                .with_properties(&[property])
                .map_err(serde::de::Error::custom)?;
        }
        Ok(Self(state))
    }
}
impl Serialize for BlockStateCodec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TextBlockState {
            name: format!("minecraft:{}", self.0.to_block().name),
            properties: self
                .0
                .properties()
                .map(|property| {
                    (
                        property.key_str().to_owned(),
                        property.value_str().to_owned(),
                    )
                })
                .collect(),
        }
        .serialize(serializer)
    }
}
impl BlockStateCodec {
    /// Resolve generated textual configuration once, then drop its temporary map.
    pub fn from_text_properties(block: &Block, properties: HashMap<String, String>) -> Self {
        let mut id = block.default_state.id;
        for (key, value) in properties {
            let property =
                Property::from_str_pair(&key, &value).expect("Unknown configured block property");
            id = id
                .with_properties(&[property])
                .expect("Invalid configured block property");
        }
        Self(id)
    }

    pub const fn new(state: BlockStateId) -> Self {
        Self(state)
    }
    pub const fn get_state(&self) -> &'static BlockState {
        self.0.to_state()
    }
    pub const fn get_block(&self) -> &'static Block {
        self.0.to_block()
    }
    pub const fn get_state_id(&self) -> BlockStateId {
        self.0
    }
}
