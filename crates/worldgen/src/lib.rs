#![allow(clippy::unreachable)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod biome;
pub mod block;
pub mod chunk;
pub mod chunk_system;
pub mod generation;
pub mod lighting;
pub mod tick;
pub mod world;

pub const CURRENT_MC_VERSION: &str = "26.3";

pub use generation::{GlobalRandomConfig, proto_chunk::ProtoChunk};
mod embedding;
pub use embedding::*;
pub mod block_relations;
pub mod blocks;
pub use hollow_core::{coordinates, sections};
pub mod state;
pub use blocks::VoxelKind;
pub use state::VoxelState;
pub mod codec;
pub mod reconcile;
pub mod runtime;
pub mod tasks;
pub use runtime::{GenerationSession, GenerationSettings, WorldGenerationPlugin};
