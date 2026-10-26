//! Blocking persistence contract. Absence and read failure are distinct outcomes.
use crate::codec::GenerationRecord;
use std::io;
pub trait ChunkStorage: Send + Sync + 'static {
    fn read_chunks(&self, positions: &[(i32, i32)]) -> io::Result<Vec<Option<GenerationRecord>>>;
    fn write_chunks(&self, chunks: Vec<((i32, i32), GenerationRecord)>) -> io::Result<()>;
    fn flush(&self) -> io::Result<()>;
}
