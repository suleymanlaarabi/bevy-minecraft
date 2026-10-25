import bevy_pbr::render::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
};

// Brightness in X, from 0 by day to 1 at night.
@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> settings: vec4<f32>;

struct StarVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) corner: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct StarVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vertex(in: StarVertex) -> StarVaryings {
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let direction = normalize((world_from_local * vec4<f32>(in.position, 0.0)).xyz);
    var out: StarVaryings;
    // A direction (w = 0) projects to the far plane at infinity: depth 0 in reverse Z,
    // so any terrain or cloud in front hides the star.
    out.position = view.clip_from_world * vec4<f32>(direction, 0.0);
    // Extinction near the horizon, where the air is thickest.
    let horizon = smoothstep(-0.02, 0.3, direction.y);
    out.corner = in.corner;
    out.color = vec4<f32>(in.color.rgb * (horizon * settings.x), in.color.a);
    return out;
}

@fragment
fn fragment(in: StarVaryings) -> @location(0) vec4<f32> {
    let falloff = max(1.0 - dot(in.corner, in.corner), 0.0);
    let seed = in.color.a;
    let twinkle = 0.7 + 0.3 * sin(globals.time * (1.5 + 3.0 * seed) + 40.0 * seed);
    // Additive: premultiplied color with zero alpha.
    return vec4<f32>(in.color.rgb * (falloff * falloff * twinkle), 0.0);
}
