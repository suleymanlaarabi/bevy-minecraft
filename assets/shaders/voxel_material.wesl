import super::block_lighting::{sky_visibility, block_irradiance};
import super::voxel_fog::apply_voxel_fog;
import super::voxel_vertex::{PackedVertex, decode, face_normal, block_uv, face_index, transform_uv, face_layer, face_transform};
import bevy_pbr::render::{
    forward_io::FragmentOutput,
    mesh_functions,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows,
    view_transformations::position_world_to_clip,
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
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) light: vec2<f32>,
    @location(4) @interpolate(flat) layer: u32,
    @location(5) @interpolate(flat) tint: vec3<f32>,
}

@vertex
fn vertex(in: PackedVertex) -> VoxelVaryings {
    let vertex = decode(in.voxel);
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var out: VoxelVaryings;
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        face_normal(vertex.direction),
        in.instance_index,
    );
    out.light = vertex.light;
    // Constant over a quad, whose four vertices share appearance and direction.
    let face = faces[face_index(vertex)];
    let packed = u32(face.a + 0.5);
    out.uv = transform_uv(block_uv(vertex.position, vertex.direction >> 1u), face_transform(packed));
    out.layer = face_layer(packed);
    out.tint = face.rgb;
    return out;
}

@fragment
fn fragment(
    in: VoxelVaryings,
) -> FragmentOutput {
    var texture_color = textureSample(blocks, blocks_sampler, in.uv, in.layer);
@if(VOXEL_CUTOUT) {
    // Leaves and other cutout cubes: real holes, alpha-tested like vanilla.
    if texture_color.a < 0.5 { discard; }
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
    let normal = normalize(in.world_normal);
    let voxel_light = in.light;
    let sky = sky_visibility(voxel_light);
    var irradiance = view_bindings::lights.ambient_color.rgb * sky
        + block_irradiance(voxel_light);

    if view_bindings::lights.n_directional_lights > 0u {
        let light = &view_bindings::lights.directional_lights[0u];
        let ndotl = max(dot(normal, (*light).direction_to_light), 0.0);
        var shadow = 1.0;
        if (((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            let view_z = dot(
                vec4<f32>(
                    view_bindings::view.view_from_world[0].z,
                    view_bindings::view.view_from_world[1].z,
                    view_bindings::view.view_from_world[2].z,
                    view_bindings::view.view_from_world[3].z,
                ),
                in.world_position,
            );
            shadow = shadows::fetch_directional_shadow(
                0u,
                in.world_position,
                normal,
                view_z,
                in.position.xy,
            );
        }
        irradiance += (*light).color.rgb * (ndotl * shadow * sky * 0.318309886);
    }

    var out: FragmentOutput;
    out.color = apply_voxel_fog(
        vec4<f32>(albedo * irradiance * view_bindings::view.exposure, 1.0),
        in.world_position.xyz,
    );
    return out;
}
