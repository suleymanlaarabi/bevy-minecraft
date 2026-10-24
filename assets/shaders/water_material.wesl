import bevy_pbr::render::{
    mesh_view_bindings::globals,
    pbr_fragment::pbr_input_from_standard_material,
};
@if(PREPASS_PIPELINE)
import bevy_pbr::{
    prepass::io::{VertexOutput, FragmentOutput},
    deferred::functions::deferred_output,
};
@if(!PREPASS_PIPELINE)
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
@if(!PREPASS_PIPELINE)
import super::voxel_lighting::apply_voxel_lighting;
@if(!PREPASS_PIPELINE)
import super::voxel_fog::apply_voxel_fog;
@if(!PREPASS_PIPELINE)
import super::block_lighting::sky_visibility;

struct WaterSettings {
    tint: vec4<f32>,
    sky_color: vec4<f32>,
    alpha: vec4<f32>,
    animation: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> water: WaterSettings;
@group(constants::MATERIAL_BIND_GROUP) @binding(101)
var still: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102)
var still_sampler: sampler;

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
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let world = pbr_input.world_position.xyz;
    // Flat like vanilla: the motion lives in the texture, not in bent normals.
    let normal = normalize(in.world_normal);
    pbr_input.N = select(-normal, normal, is_front);

    // Red vertex color is the normalized water column depth (1 on sides and undersides).
@if(VERTEX_COLORS)
    let depth = in.color.r;
@if(!VERTEX_COLORS)
    let depth = 1.0;

    let facing = clamp(dot(pbr_input.N, pbr_input.V), 0.0, 1.0);
    // Viewed from inside the water, the surface keeps a flat, slightly darker look.
    let fresnel = select(0.0, pow(1.0 - facing, 5.0), is_front);
    let albedo = water.tint.rgb * water_texture(world, normal) * mix(1.0, 0.75, depth);
    let opacity = select(water.alpha.z, mix(water.alpha.x, water.alpha.y, depth), is_front);
    pbr_input.material.base_color = vec4<f32>(
        select(albedo * 0.8, albedo, is_front),
        mix(opacity, water.alpha.w, fresnel),
    );

@if(PREPASS_PIPELINE) {
    return deferred_output(in, pbr_input);
} @else {
    var out: FragmentOutput;
@if(VERTEX_UVS_B)
    let voxel_light = in.uv_b;
@if(!VERTEX_UVS_B)
    let voxel_light = vec2<f32>(1.0, 0.0);
    let lit = apply_voxel_lighting(pbr_input, voxel_light);
    // Cheap sky reflection: the clear color is already in output space, so blend it
    // after lighting, dimmed where the sky cannot reach.
    let sky = water.sky_color.rgb * sky_visibility(voxel_light);
    out.color = vec4<f32>(mix(lit.rgb, sky, fresnel * water.sky_color.a), lit.a);
    out.color = apply_voxel_fog(out.color, world);
    return out;
}
}
