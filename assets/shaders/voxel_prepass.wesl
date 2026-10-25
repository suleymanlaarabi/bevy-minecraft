import super::voxel_vertex::{PackedVertex, decode, block_uv, face_index, transform_uv, face_layer, face_transform};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var blocks: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1)
var blocks_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2)
var<storage, read> faces: array<vec4<f32>>;

struct PrepassVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
    @location(2) @interpolate(linear) unclipped_depth: f32,
}

// Shadow maps and depth prepasses need the position of packed terrain; cutout
// cubes also need their texture to discard the holes.
@vertex
fn vertex(in: PackedVertex) -> PrepassVaryings {
    let vertex = decode(in.voxel);
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    var out: PrepassVaryings;
    out.position = position_world_to_clip(world_position.xyz);
    out.unclipped_depth = out.position.z;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    // Clamp depth to avoid clipping.
    out.position.z = min(out.position.z, 1.0);
}
    out.uv = vec2<f32>(0.0);
    out.layer = 0u;
@if(VOXEL_CUTOUT) {
    let packed = u32(faces[face_index(vertex)].a + 0.5);
    out.uv = transform_uv(block_uv(vertex.position, vertex.direction >> 1u), face_transform(packed));
    out.layer = face_layer(packed);
}
    return out;
}

fn discard_holes(in: PrepassVaryings) {
@if(VOXEL_CUTOUT) {
    if textureSample(blocks, blocks_sampler, in.uv, in.layer).a < 0.5 { discard; }
}
}

// Only compiled when Bevy needs a fragment stage: alpha-tested cutouts or
// emulated unclipped depth in shadow maps. Depth is written only for the
// emulation, so plain cutouts keep early and hierarchical depth tests.
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
@fragment
fn fragment(in: PrepassVaryings) -> @builtin(frag_depth) f32 {
    discard_holes(in);
    return in.unclipped_depth;
}

@if(!UNCLIPPED_DEPTH_ORTHO_EMULATION)
@fragment
fn fragment(in: PrepassVaryings) {
    discard_holes(in);
}
