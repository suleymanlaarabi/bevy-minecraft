//! Shared checked compression and optional transactional persistence.
use std::io;
#[cfg(feature = "sqlite")]
pub mod sqlite;
pub fn compress_record(bytes: &[u8]) -> Vec<u8> {
    let mut record = crc32fast::hash(bytes).to_le_bytes().to_vec();
    record.extend_from_slice(&lz4_flex::compress_prepend_size(bytes));
    record
}
pub fn decompress_record(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let checksum = bytes
        .get(..4)
        .ok_or_else(|| io::Error::other("Invalid compressed terrain record"))?;
    let compressed = &bytes[4..];
    let size = compressed
        .get(..4)
        .ok_or_else(|| io::Error::other("Invalid compressed terrain record"))?;
    if u32::from_le_bytes(size.try_into().unwrap()) as usize > 8 * 1024 * 1024 {
        return Err(io::Error::other("Invalid compressed terrain record"));
    }
    let record = lz4_flex::decompress_size_prepended(compressed).map_err(io::Error::other)?;
    if crc32fast::hash(&record) != u32::from_le_bytes(checksum.try_into().unwrap()) {
        return Err(io::Error::other("Invalid compressed terrain record"));
    }
    Ok(record)
}
