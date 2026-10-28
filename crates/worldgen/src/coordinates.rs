//! Native generation geometry. Shared runtime geometry is supplied by WorldLayout.
pub use hollow_core::coordinates::*;
pub const CHUNK_HEIGHT: i32 = 384;
pub const SECTION_COUNT: usize = (CHUNK_HEIGHT / SECTION_HEIGHT) as usize;
