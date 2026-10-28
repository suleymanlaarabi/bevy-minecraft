import super::sky_color::sample_sky;
import bevy_pbr::render::mesh_view_bindings::view;

struct SkyVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
}

@vertex
fn vertex(@location(0) position: vec3<f32>) -> SkyVaryings {
    var out: SkyVaryings;
    out.position = vec4<f32>(position.xy, 0.0, 1.0);
    let world = view.world_from_clip * out.position;
    out.direction = world.xyz - view.world_position.xyz * world.w;
    return out;
}

@fragment
fn fragment(in: SkyVaryings) -> @location(0) vec4<f32> {
    return vec4<f32>(sample_sky(normalize(in.direction)), 1.0);
}
