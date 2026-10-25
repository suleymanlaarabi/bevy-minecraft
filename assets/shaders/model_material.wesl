import super::block_lighting::{sky_visibility, block_irradiance};
import super::voxel_fog::apply_voxel_fog;
import super::model_vertex::{ModelVertex, ModelVaryings, decode_model};
import super::voxel_lighting::apply_voxel_lighting;
import bevy_pbr::render::{
    forward_io::FragmentOutput,
    mesh_functions,
    mesh_view_bindings::{globals, view},
    pbr_functions::calculate_view,
    pbr_types::pbr_input_new,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows,
    view_transformations::position_world_to_clip,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var blocks: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1)
var blocks_sampler: sampler;
// Tint in RGB and texture layer in A, indexed by model slot, then the tint
// palette. Read once per vertex: a per-fragment lookup would put a dependent
// storage read before every texture sample.
@group(constants::MATERIAL_BIND_GROUP) @binding(2)
var<storage, read> faces: array<vec4<f32>>;

struct WaterSettings {
    tint: vec4<f32>,
    sky_color: vec4<f32>,
    alpha: vec4<f32>,
    animation: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(3)
var<uniform> water: WaterSettings;
@group(constants::MATERIAL_BIND_GROUP) @binding(4)
var still: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5)
var still_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(6) var cracks: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var cracks_sampler: sampler;

@vertex
fn vertex(in: ModelVertex) -> ModelVaryings {
    let model = decode_model(in.data);
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var out: ModelVaryings;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(model.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(model.normal, in.instance_index);
    out.uv = model.uv;
    out.light = model.light;
    let face = faces[model.slot];
    out.layer = u32(face.a + 0.5);
    out.tint = face.rgb;
    // Palette entry 0 is white: most vertices skip the second storage read.
    if model.tint != 0u {
        out.tint *= faces[arrayLength(&faces) - 8u + model.tint].rgb;
    }
    out.flags = model.flags;
    out.depth = f32((model.flags >> 1u) & 15u) / 15.0;
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

fn shade_water(
    in: ModelVaryings,
    is_front: bool,
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
    // Cheap sky reflection: the clear color is already in output space, so blend it
    // after lighting, dimmed where the sky cannot reach.
    let sky = water.sky_color.rgb * sky_visibility(in.light);
    var out: FragmentOutput;
    out.color = apply_voxel_fog(
        vec4<f32>(mix(lit.rgb, sky, fresnel * water.sky_color.a), lit.a),
        world,
    );
    return out;
}
@fragment
fn fragment(
    in: ModelVaryings,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    // The water/material flag varies per primitive. Compute derivatives before
    // that branch so texture sampling remains valid in divergent control flow.
    let uv_dx = dpdx(in.uv);
    let uv_dy = dpdy(in.uv);
    if (in.flags & 1u) != 0u { return shade_water(in, is_front); }
    let texture_color = textureSampleGrad(blocks, blocks_sampler, in.uv, in.layer, uv_dx, uv_dy);
    // Opaque pipelines never discard, so the GPU keeps early depth tests. Only the
    // double-sided translucent pipeline sees back faces.
@if(MODEL_BLEND) {
    if !is_front { discard; }
}
@if(MODEL_BLEND || MODEL_BREAKING) {
    if texture_color.a < 0.004 { discard; }
}
@if(MODEL_CUTOUT) {
    if texture_color.a < 0.5 { discard; }
}
@if(MODEL_BREAKING) {
    let crack = textureSampleGrad(cracks, cracks_sampler, in.uv, uv_dx, uv_dy);
    var breaking_out: FragmentOutput;
    // Bevy's Multiply blend is dst * src.rgb + dst * (1 - src.a).
    // Premultiply so transparent texels leave the lit block unchanged and
    // partially covered texels can only darken it.
    breaking_out.color = vec4<f32>(crack.rgb * crack.a, crack.a);
    return breaking_out;
}
    let albedo = texture_color.rgb * in.tint;
    let normal = normalize(select(-in.world_normal, in.world_normal, is_front));
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
        vec4<f32>(albedo * irradiance * view_bindings::view.exposure, texture_color.a),
        in.world_position.xyz,
    );
    return out;
}
