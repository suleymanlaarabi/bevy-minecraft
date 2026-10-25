import super::block_lighting::celestial_irradiance;
import bevy_pbr::render::{
    forward_io::FragmentOutput,
    mesh_functions,
    mesh_view_bindings as view_bindings,
    view_transformations::position_world_to_clip,
};

// Horizontal distances where the dithered fade starts and ends in XY.
@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> fade: vec4<f32>;

struct CloudVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct CloudVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) @interpolate(flat) normal: vec3<f32>,
}

@vertex
fn vertex(in: CloudVertex) -> CloudVaryings {
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    var out: CloudVaryings;
    // The layer is only ever translated: local normals are world normals.
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(in.position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
    out.normal = in.normal;
    return out;
}

// Jimenez's interleaved gradient noise: a stable per-pixel threshold.
fn dither(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fragment(in: CloudVaryings) -> FragmentOutput {
    // Opaque clouds dissolve toward the edge of the layer instead of blending, so
    // the atmosphere still applies its aerial perspective to them afterwards.
    let distance = length(in.world_position.xz - view_bindings::view.world_position.xz);
    if 1.0 - smoothstep(fade.x, fade.y, distance) <= dither(in.position.xy) {
        discard;
    }
    // Vanilla's face shading on top of real light: sunlit tops, darker sides and
    // undersides lit by the sky alone.
    let n = in.normal;
    let face = select(select(0.8, 0.9, abs(n.x) > 0.5), select(0.72, 1.0, n.y > 0.0), abs(n.y) > 0.5);
    let irradiance = view_bindings::lights.ambient_color.rgb
        + celestial_irradiance(n, in.world_position, in.position.xy, false);
    var out: FragmentOutput;
    out.color = vec4<f32>(
        vec3<f32>(0.96) * face * irradiance * view_bindings::view.exposure,
        1.0,
    );
    return out;
}
