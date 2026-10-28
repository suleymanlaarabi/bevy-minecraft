import bevy_pbr::{
    prepass::io::{Vertex, VertexOutput, decompress_vertex},
    render::{mesh_functions, view_transformations::position_world_to_clip},
};

@vertex
fn vertex(input: Vertex) -> VertexOutput {
    let v = decompress_vertex(input, input.instance_index);
    let tag = mesh_functions::get_tag(input.instance_index);
    let swing = f32(tag & 65535u) * (2.0 / 65535.0) - 1.0;
    let bob = (f32((tag >> 16u) & 255u) * (2.0 / 255.0) - 1.0) * 0.025;
    let angle = swing * v.color.w;
    let c = cos(angle);
    let s = sin(angle);
    let relative = v.position - v.color.xyz;
    let leg_position = vec3<f32>(relative.x, c * relative.y - s * relative.z, s * relative.y + c * relative.z) + v.color.xyz;
    let roll = f32(tag >> 24u) * (1.57079632679 / 255.0);
    let rc = cos(roll);
    let rs = sin(roll);
    let feet = vec3<f32>(0.0, -v.uv_b.x, 0.0);
    let body = leg_position - feet;
    let local = vec3<f32>(rc * body.x - rs * body.y, rs * body.x + rc * body.y, body.z)
        + feet + vec3<f32>(0.0, bob, 0.0);
    let world = mesh_functions::get_world_from_local(input.instance_index);
    var out: VertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(world, vec4<f32>(local, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
}
@if(VERTEX_UVS_A)
    out.uv = v.uv;
@if(VERTEX_UVS_B)
    out.uv_b = v.uv_b;
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS) {
    let leg_normal = vec3<f32>(v.normal.x, c * v.normal.y - s * v.normal.z, s * v.normal.y + c * v.normal.z);
    let normal = vec3<f32>(rc * leg_normal.x - rs * leg_normal.y, rs * leg_normal.x + rc * leg_normal.y, leg_normal.z);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(normal, input.instance_index);
}
@if(VERTEX_COLORS)
    out.color = vec4<f32>(1.0);
@if(MOTION_VECTOR_PREPASS) {
    let previous = mesh_functions::get_previous_world_from_local(input.instance_index);
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(previous, vec4<f32>(local, 1.0));
}
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = input.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(input.instance_index, world[3]);
    return out;
}
