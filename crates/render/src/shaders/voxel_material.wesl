import super::section_transform;
import super::surface_animation::surface_frame;
import super::block_lighting::voxel_diffuse;
import super::voxel_fog::{apply_voxel_fog_ray, fog_ray, fully_fogged};
import super::voxel_vertex::{PackedVertex, decode, face_normal, block_uv, face_index, transform_uv, face_layer, face_transform};
import bevy_pbr::render::{
    forward_io::FragmentOutput,
    mesh_functions,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var blocks: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1)
var blocks_sampler: sampler;
// Tint in RGB and packed layer/UV transform in A, indexed by `face_index`. Read once per vertex:
// a per-fragment lookup would put a dependent storage read before every texture sample.
@group(constants::MATERIAL_BIND_GROUP) @binding(2)
var<storage, read> faces: array<vec4<f32>>;

struct VoxelVaryings {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) light: vec2<f32>,
    @location(4) @interpolate(flat) layer: u32,
    @location(5) @interpolate(flat) tint: vec3<f32>,
    @location(6) @interpolate(flat) next_layer: u32,
    @location(7) @interpolate(flat) frame_mix: f32,
}

@vertex
fn vertex(in: PackedVertex) -> VoxelVaryings {
    let vertex = decode(in.voxel);
    let world_from_local = section_transform::world_from_local(in.instance_index);
    var out: VoxelVaryings;
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    // Use exactly the same transform and invariant position as the depth prepass.
    out.position = mesh_functions::mesh_position_local_to_clip(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.world_normal = section_transform::world_normal(
        face_normal(vertex.direction),
        in.instance_index,
    );
    out.light = vertex.light;
    // Constant over a quad, whose four vertices share appearance and direction.
    let face = faces[face_index(vertex)];
    let packed = u32(face.a + 0.5);
    out.uv = transform_uv(block_uv(vertex.position, vertex.direction >> 1u), face_transform(packed));
    let frame = surface_frame(face_layer(packed));
    out.layer = frame.current;
    out.next_layer = frame.next;
    out.frame_mix = frame.blend;
    out.tint = face.rgb;
    return out;
}

@fragment
fn fragment(
    in: VoxelVaryings,
) -> FragmentOutput {
    let uv_dx = dpdx(in.uv);
    let uv_dy = dpdy(in.uv);
    let ray = fog_ray(in.world_position.xyz);
    var out: FragmentOutput;
@if(!VOXEL_CUTOUT) {
    if fully_fogged(ray) {
        out.color = apply_voxel_fog_ray(vec4<f32>(0.0, 0.0, 0.0, 1.0), in.world_position.xyz, ray);
        return out;
    }
}
    var texture_color = textureSampleGrad(blocks, blocks_sampler, in.uv, in.layer, uv_dx, uv_dy);
    if in.frame_mix > 0.0 {
        texture_color = mix(texture_color, textureSampleGrad(blocks, blocks_sampler, in.uv, in.next_layer, uv_dx, uv_dy), in.frame_mix);
    }
@if(VOXEL_CUTOUT) {
    // Leaves and other cutout cubes: real holes, alpha-tested like vanilla.
    if texture_color.a < 0.5 { discard; }
    if fully_fogged(ray) {
        out.color = apply_voxel_fog_ray(vec4<f32>(0.0, 0.0, 0.0, 1.0), in.world_position.xyz, ray);
        return out;
    }
} @else {
    // Opaque terrain: close stray cutouts (any layer with alpha) with a dark tone.
    if texture_color.a < 1.0 {
        texture_color = vec4<f32>(
            mix(vec3<f32>(0.32), texture_color.rgb, texture_color.a),
            1.0,
        );
    }
}
    let albedo = texture_color.rgb * in.tint;
    out.color = apply_voxel_fog_ray(
        vec4<f32>(
            voxel_diffuse(albedo, normalize(in.world_normal), in.world_position, in.position.xy, in.light),
            1.0,
        ),
        in.world_position.xyz, ray,
    );
    return out;
}
