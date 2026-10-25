import super::voxel_vertex::{PackedVertex, decode, face_normal};
import super::voxel_lighting::apply_voxel_lighting;
import super::voxel_fog::{apply_voxel_fog, sky_reflection};
import super::block_lighting::sky_visibility;
import bevy_pbr::render::{
    forward_io::FragmentOutput,
    mesh_functions,
    mesh_view_bindings::{globals, view},
    pbr_functions::calculate_view,
    pbr_types::pbr_input_new,
    view_transformations::position_world_to_clip,
};

struct WaterSettings {
    tint: vec4<f32>,
    sky_color: vec4<f32>,
    alpha: vec4<f32>,
    animation: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> water: WaterSettings;
@group(constants::MATERIAL_BIND_GROUP) @binding(1)
var still: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2)
var still_sampler: sampler;

// Vanilla still water stops 2/16 below the top of its block.
const SURFACE_DROP: f32 = 2.0 / 16.0;

struct WaterVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) light: vec2<f32>,
    // Normalized water column depth (1 on sides and undersides).
    @location(3) depth: f32,
}

// Word 1 of a water vertex also holds the corner depth (bits 12..16) and whether the
// corner sits at the lowered surface level (bit 16), see `meshing::water_mesh`.
@vertex
fn vertex(in: PackedVertex) -> WaterVaryings {
    let vertex = decode(in.voxel);
    var position = vertex.position;
    position.y -= f32((in.voxel.y >> 16u) & 1u) * SURFACE_DROP;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var out: WaterVaryings;
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        face_normal(vertex.direction),
        in.instance_index,
    );
    out.light = vertex.light;
    out.depth = f32((in.voxel.y >> 12u) & 15u) / 15.0;
    return out;
}

// Integer hash: stable at any world coordinate, unlike fract(sin()) hashes.
fn hash(p: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 0x8da6b343u)
        ^ (bitcast<u32>(p.y) * 0xd8163841u)
        ^ (bitcast<u32>(p.z) * 0xcb1ab31fu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h) * (1.0 / 4294967295.0);
}

fn value_noise(p: vec3<f32>) -> f32 {
    let cell = vec3<i32>(floor(p));
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let x00 = mix(hash(cell), hash(cell + vec3<i32>(1, 0, 0)), w.x);
    let x10 = mix(hash(cell + vec3<i32>(0, 1, 0)), hash(cell + vec3<i32>(1, 1, 0)), w.x);
    let x01 = mix(hash(cell + vec3<i32>(0, 0, 1)), hash(cell + vec3<i32>(1, 0, 1)), w.x);
    let x11 = mix(hash(cell + vec3<i32>(0, 1, 1)), hash(cell + vec3<i32>(1, 1, 1)), w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}

// Linear grey of the still water texture, projected on the face plane.
fn water_texture(world: vec3<f32>, normal: vec3<f32>) -> f32 {
    let axis = abs(normal);
    var plane = world.xz;
    if axis.x > 0.5 {
        plane = vec2<f32>(world.z, -world.y);
    } else if axis.z > 0.5 {
        plane = vec2<f32>(world.x, -world.y);
    }
    let frames = water.animation.x;
    if frames > 0.0 {
        // Vanilla's flipbook, cross-faded between frames so it flows instead of stepping.
        let time = globals.time * water.animation.y;
        let frame = floor(time);
        let uv = fract(plane);
        let current = textureSampleLevel(still, still_sampler, uv, i32(frame % frames), 0.0).r;
        let next = textureSampleLevel(still, still_sampler, uv, i32((frame + 1.0) % frames), 0.0).r;
        return mix(current, next, time - frame);
    }
    // Without imported textures: a quiet, smoothly drifting texel pattern with the
    // flipbook's average brightness.
    let texel = floor(plane * 16.0) + 0.5;
    let time = globals.time * 0.6;
    let shade = value_noise(vec3<f32>(texel / 6.0, time))
        * 0.6 + value_noise(vec3<f32>(texel / 3.0 + 31.0, time * 1.3)) * 0.4;
    return 0.36 + shade * 0.22;
}

@fragment
fn fragment(
    in: WaterVaryings,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let world = in.world_position.xyz;
    // Flat like vanilla: the motion lives in the texture, not in bent normals.
    let normal = normalize(in.world_normal);

    // The glossy dielectric the water used to get from its StandardMaterial base.
    // Water never receives shadows (`NotShadowReceiver`), hence no mesh flags.
    var pbr_input = pbr_input_new();
    pbr_input.is_orthographic = view.clip_from_view[3].w == 1.0;
    pbr_input.V = calculate_view(in.world_position, pbr_input.is_orthographic);
    pbr_input.frag_coord = in.position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = in.world_normal;
    pbr_input.N = select(-normal, normal, is_front);
    pbr_input.material.perceptual_roughness = 0.08;

    let facing = clamp(dot(pbr_input.N, pbr_input.V), 0.0, 1.0);
    // Viewed from inside the water, the surface keeps a flat, slightly darker look.
    let fresnel = select(0.0, pow(1.0 - facing, 5.0), is_front);
    let albedo = water.tint.rgb * water_texture(world, normal) * mix(1.0, 0.75, in.depth);
    let opacity = select(water.alpha.z, mix(water.alpha.x, water.alpha.y, in.depth), is_front);
    pbr_input.material.base_color = vec4<f32>(
        select(albedo * 0.8, albedo, is_front),
        mix(opacity, water.alpha.w, fresnel),
    );

    let lit = apply_voxel_lighting(pbr_input, in.light);
    // Cheap sky reflection: the horizon color is already in output space, so blend
    // it after lighting, dimmed where the sky cannot reach.
    let sky = sky_reflection(water.sky_color.rgb) * sky_visibility(in.light);
    var out: FragmentOutput;
    out.color = apply_voxel_fog(
        vec4<f32>(mix(lit.rgb, sky, fresnel * water.sky_color.a), lit.a),
        world,
    );
    return out;
}
