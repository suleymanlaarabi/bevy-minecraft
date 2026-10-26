//! Generation stage rules and working neighborhoods shared with the Bevy runtime.
pub type ChunkPos = pumpkin_util::math::vector2::Vector2<i32>;
pub mod chunk_state;
pub mod generation;
pub mod generation_cache;
pub use chunk_state::{Chunk, StagedChunkEnum};
pub use generation::generate_single_chunk;
pub use generation_cache::Cache;
