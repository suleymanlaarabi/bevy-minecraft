//! Biome-source settings consumed by the generation algorithms.
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum BiomeSource {
    WithPreset {
        preset: String,
        #[serde(rename = "type")]
        biome_type: String,
    },
    Fixed {
        biome: String,
        #[serde(rename = "type")]
        biome_type: String,
    },
    Simple {
        #[serde(rename = "type")]
        biome_type: String,
    },
}
