import super::voxel_vertex::{PackedVertex, decode, face_normal};
import bevy_pbr::{
    prepass::io::VertexOutput,
    render::{mesh_functions, view_transformations::position_world_to_clip},
};

// Shadow maps and depth prepasses only need the position of packed terrain.
@vertex
fn vertex(in: PackedVertex) -> VertexOutput {
    let vertex = decode(in.voxel);
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var out: VertexOutput;
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.unclipped_depth = out.position.z;
    // Clamp depth to avoid clipping.
    out.position.z = min(out.position.z, 1.0);
}
@if(NORMAL_PREPASS_OR_DEFERRED_PREPASS)
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        face_normal(vertex.direction),
        in.instance_index,
    );
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = in.instance_index;
    return out;
}
