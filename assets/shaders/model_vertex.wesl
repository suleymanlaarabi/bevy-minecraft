import bevy_render::utils::octahedral_decode_signed;
// Decodes the packed model vertex written by `model_mesh::pack_vertex`.
struct ModelVertex {
    @builtin(instance_index) instance_index: u32,
    @builtin(vertex_index) vertex_index: u32,
    @location(0) data: vec4<u32>,
}
struct ModelVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) light: vec2<f32>,
    @location(4) @interpolate(flat) layer: u32,
    @location(5) @interpolate(flat) tint: vec3<f32>,
    @location(6) @interpolate(flat) flags: u32,
    @location(7) depth: f32,
    @location(8) @interpolate(flat) next_layer: u32,
    @location(9) @interpolate(flat) frame_mix: f32,
    @if(MODEL_BREAKING) @location(10) crack_uv: vec2<f32>,
}

struct DecodedModel {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    // Model face-table slot.
    slot: u32,
    // Normalized sky and block light.
    light: vec2<f32>,
    // Index into the tint palette stored after the face table.
    tint: u32,
    // Bit 0: water; bits 1-4: water column depth.
    flags: u32,
}

const POSITION_BIAS: f32 = 2.0;
const POSITION_SCALE: f32 = 1024.0;

fn decode_position(data: vec4<u32>) -> vec3<f32> {
    let fixed = vec3<u32>(
        data.x & 32767u,
        (data.x >> 15u) & 32767u,
        (data.x >> 30u) | ((data.y & 8191u) << 2u),
    );
    return vec3<f32>(fixed) / POSITION_SCALE - POSITION_BIAS;
}

fn decode_model(data: vec4<u32>) -> DecodedModel {
    var out: DecodedModel;
    out.position = decode_position(data);
    let normal = vec2<u32>(data.y >> 13u, data.y >> 21u) & vec2<u32>(255u);
    out.normal = octahedral_decode_signed(vec2<f32>(normal) / 254.0 * 2.0 - 1.0);
    out.tint = data.y >> 29u;
    out.uv = vec2<f32>(vec2<u32>(data.z, data.z >> 16u) & vec2<u32>(65535u)) / 4096.0 - 8.0;
    out.slot = data.w & 8191u;
    let light = (data.w >> 13u) & 4095u;
    out.light = vec2<f32>(vec2<u32>(light, light >> 6u) & vec2<u32>(63u)) / 60.0;
    out.flags = (data.w >> 25u) & 31u;
    return out;
}
