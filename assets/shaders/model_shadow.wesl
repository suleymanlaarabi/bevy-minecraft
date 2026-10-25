import super::model_vertex::{ModelVertex, decode_model};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var blocks: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var blocks_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<storage, read> faces: array<vec4<f32>>;
struct ShadowVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) @interpolate(linear) unclipped_depth: f32,
}
@vertex
fn vertex(in: ModelVertex) -> ShadowVaryings {
    var out: ShadowVaryings;
    let transform = mesh_functions::get_world_from_local(in.instance_index);
    let model = decode_model(in.data);
    let world = mesh_functions::mesh_position_local_to_world(transform, vec4<f32>(model.position, 1.0));
    out.position = position_world_to_clip(world.xyz);
    out.unclipped_depth = out.position.z;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.position.z = min(out.position.z, 1.0);
}
    out.uv = model.uv;
    out.layer = u32(faces[model.slot].a + 0.5);
    return out;
}
@fragment
fn fragment(in: ShadowVaryings) -> @builtin(frag_depth) f32 {
@if(MAY_DISCARD) {
    if textureSample(blocks, blocks_sampler, in.uv, in.layer).a < 0.5 { discard; }
}
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    return in.unclipped_depth;
} @else {
    return in.position.z;
}
}
