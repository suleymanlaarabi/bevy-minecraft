//! Shared checked compression and optional transactional persistence.
use std::io;
#[cfg(feature = "sqlite")]
pub mod sqlite;
/// Checksum, size prefix and LZ4 block share one buffer: no separate block
/// allocation is compressed into and then copied.
pub fn compress_record(bytes: &[u8]) -> Vec<u8> {
    let mut record = vec![0; 8 + lz4_flex::block::get_maximum_output_size(bytes.len())];
    record[..4].copy_from_slice(&crc32fast::hash(bytes).to_le_bytes());
    record[4..8].copy_from_slice(
        &u32::try_from(bytes.len())
            .expect("Record exceeds the LZ4 size prefix")
            .to_le_bytes(),
    );
    let size = lz4_flex::block::compress_into(bytes, &mut record[8..])
        .expect("Output sized for the LZ4 worst case");
    record.truncate(8 + size);
    // Records stay queued for SQL in batches; release the worst-case reserve.
    record.shrink_to_fit();
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
