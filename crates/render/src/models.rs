use bevy::prelude::*;
use serde::Deserialize;
#[derive(Clone, Debug, Deserialize)]
pub struct ModelQuad {
    #[serde(deserialize_with = "model_positions")]
    pub positions: [[f32; 3]; 4],
    pub normal: [f32; 3],
    pub uv: [[f32; 2]; 4],
    pub texture: u32,
    pub tinted: bool,
    pub cull: [i32; 3],
    pub translucent: bool,
    /// Non-water surface effect flags: bit 1 is the end-portal surface.
    #[serde(default)]
    pub effect: u8,
    #[serde(default)]
    pub light_emission: u8,
    /// Vanilla's `shade_direction_override: "up"`, resolved once instead of per quad.
    #[serde(default, rename = "shade", deserialize_with = "shade_up")]
    pub shade_up: bool,
}

fn model_positions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[[f32; 3]; 4], D::Error> {
    let mut positions = <[[f32; 3]; 4]>::deserialize(deserializer)?;
    let step = 1.0 / crate::model_mesh::POSITION_SCALE;
    for position in &mut positions {
        *position = position.map(|value| {
            // Models place thin faces just inside their pixel-grid planes (roots
            // use 0.000125 blocks). Keep at least one packed position step between
            // them and the plane, without moving exact planes or rotation noise.
            // Resolve this once at import; mesh workers retain 16-byte vertices.
            let plane = (value * 16.0).round() / 16.0;
            let offset = value - plane;
            if (1e-5..step * 0.5).contains(&offset.abs()) {
                plane + offset.signum() * step
            } else {
                value
            }
        });
    }
    Ok(positions)
}

fn shade_up<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(String::deserialize(deserializer)? == "up")
}
/// Per-state mesher flags (bits of [`CellInfo::flags`]).
pub mod cell {
    pub const AIR: u8 = 1;
    pub const OCEAN_SURFACE: u8 = 1 << 1;
    /// Opaque full cube: hides neighbor faces and darkens corners.
    pub const OCCLUDES: u8 = 1 << 2;
    /// Opaque cube drawn by the greedy terrain mesher.
    pub const CUBE: u8 = 1 << 3;
    /// Alpha-tested cube (leaves) drawn by the greedy mesher on its own mesh.
    pub const CUTOUT: u8 = 1 << 4;
    /// Explicit model quads.
    pub const MODEL: u8 = 1 << 5;
    /// Full cube: bounds water without necessarily hiding block faces or light.
    pub const FLUID_OCCLUDES: u8 = 1 << 6;
    /// Partial solid shape: clip fluid faces against its native outline boxes.
    pub const FLUID_OCCLUDER: u8 = 1 << 7;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellInfo {
    /// Greedy cube appearance (0 when not a cube), packed in 14 vertex bits.
    pub appearance: u16,
    pub flags: u8,
    /// Amount in bits 0..8, FluidId in bits 8..24 and falling in bit 24.
    /// The eight-byte record covers all registered fluid families without truncation.
    pub fluid: u32,
}

const _: () = assert!(std::mem::size_of::<CellInfo>() == 8);
