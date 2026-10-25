import super::surface_animation::surface_frame;
import super::model_vertex::{ModelVertex, decode_model};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var blocks: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var blocks_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<storage, read> faces: array<vec4<f32>>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var special: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var special_sampler: sampler;
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
    // Effect surfaces share section meshes without casting solid model shadows.
    if (model.flags & 8u) != 0u { out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    out.unclipped_depth = out.position.z;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.position.z = min(out.position.z, 1.0);
}
    out.uv = model.uv;
    out.layer = surface_frame(u32(faces[model.slot].a + 0.5)).current;
    return out;
}
@fragment
fn fragment(in: ShadowVaryings) -> @builtin(frag_depth) f32 {
@if(MAY_DISCARD) {
    let uv_dx = dpdx(in.uv);
    let uv_dy = dpdy(in.uv);
    var alpha = 1.0;
    if (in.layer & 4096u) != 0u {
        alpha = textureSampleGrad(special, special_sampler, in.uv, in.layer & 4095u, uv_dx, uv_dy).a;
    } else {
        alpha = textureSampleGrad(blocks, blocks_sampler, in.uv, in.layer, uv_dx, uv_dy).a;
    }
    if alpha < 0.5 { discard; }
}
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    return in.unclipped_depth;
} @else {
    return in.position.z;
}
}
