//! Helpers for reading and writing gzip-compressed NBT data.

use crate::deserializer::NbtReadHelperJava;
use crate::{Error, Nbt, NbtCompound};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::io::{Cursor, Read, Seek, Write};

/// Reads a gzip-compressed, named NBT compound from a seekable reader.
///
/// Decompressed data is limited to 64 MiB.
pub fn read_gzip_compound_tag(input: impl Read + Seek) -> Result<NbtCompound, Error> {
    // Create a GZip decoder and directly chain it to the NBT reader
    let mut decoder = GzDecoder::new(input).take(64 * 1024 * 1024); // 64 MB limit
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf).map_err(Error::Incomplete)?;
    let mut reader = NbtReadHelperJava::new(Cursor::new(buf));

    // Read the NBT data directly from the decoder stream
    let nbt = Nbt::read(&mut reader)?;
    Ok(nbt.root_tag)
}

/// Writes a named NBT compound with gzip compression.
///
/// The root name is written as an empty string.
pub fn write_gzip_compound_tag(compound: NbtCompound, output: impl Write) -> Result<(), Error> {
    // Create a GZip encoder that writes to the output
    let mut encoder = GzEncoder::new(output, Compression::default());

    // Create an NBT wrapper and write directly to the encoder
    let nbt = Nbt::new(String::new(), compound);
    nbt.write_to_writer(&mut encoder)
        .map_err(Error::Incomplete)?;

    // Finish the encoder to ensure all data is written
    encoder.finish().map_err(Error::Incomplete)?;

    Ok(())
}

/// Serializes a named NBT compound into a gzip-compressed byte vector.
///
/// The root name is written as an empty string.
pub fn write_gzip_compound_tag_to_bytes(compound: NbtCompound) -> Result<Vec<u8>, Error> {
    let mut buffer = Vec::new();
    write_gzip_compound_tag(compound, &mut buffer)?;
    Ok(buffer)
}
