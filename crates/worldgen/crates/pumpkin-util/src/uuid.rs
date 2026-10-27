use uuid::Uuid;

/// Parses a UUID similar to the way that Java does, which makes it ideal for
/// keeping the same behaviors sometimes.
#[must_use]
pub fn parse_uuid_array(uuid: &str) -> Option<[i32; 4]> {
    // We can't directly use the uuid crate to parse UUIDs, as it parses them
    // in a different way from Java.

    if uuid.len() > 36 {
        // UUID string is too large.
        return None;
    }

    // Split by hyphen. (5 segments)
    let mut parts = uuid.split('-');
    let mut parsed_parts: [i64; 5] = [0; 5];

    for part in &mut parsed_parts {
        // If a part is empty, the parsing functions will error anyway - this is what we want.
        *part = i64::from_str_radix(parts.next()?, 16).ok()?;
    }

    if parts.next().is_some() {
        // UUIDs must have exactly 5 parts.
        return None;
    }

    let bits = [
        (parsed_parts[0] & 0xFFFFFFFF) << 32
            | (parsed_parts[1] & 0xFFFF) << 16
            | (parsed_parts[2] & 0xFFFF),
        (parsed_parts[3] & 0xFFFF) << 48 | (parsed_parts[4] & 0xFFFFFFFFFFFF),
    ];

    Some([
        (bits[0] >> 32) as i32,
        bits[0] as i32,
        (bits[1] >> 32) as i32,
        bits[1] as i32,
    ])
}

/// Parses a UUID similar to the way that Java does, which makes it ideal for
/// keeping the same behaviors sometimes.
#[must_use]
pub fn parse_uuid_vec(uuid: &str) -> Option<Vec<i32>> {
    parse_uuid_array(uuid).map(Vec::from)
}

/// Parses a UUID similar to the way that Java does, which makes it ideal for
/// keeping the same behaviors sometimes.
#[must_use]
pub fn parse_uuid(uuid: &str) -> Option<Uuid> {
    let [a, b, c, d] = parse_uuid_array(uuid)?;

    let uuid_int = (a as u32 as u128) << 96
        | (b as u32 as u128) << 64
        | (c as u32 as u128) << 32
        | (d as u32 as u128);

    Some(Uuid::from_u128(uuid_int))
}
