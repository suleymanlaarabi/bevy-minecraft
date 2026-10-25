import super::block_lighting::atmosphere_transmittance;
import bevy_pbr::render::mesh_view_bindings::view;

struct SunSettings {
    // Unit vector toward the sun in XYZ, angular radius of the disk in W.
    direction: vec4<f32>,
    // Output-space color of the disk's center in RGB.
    color: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> sun: SunSettings;

// The quad reaches a little past the disk for its soft edge.
const EXTENT: f32 = 1.2;

struct SunVaryings {
    @builtin(position) position: vec4<f32>,
    // Disk coordinates: the sun is the unit circle.
    @location(0) disk: vec2<f32>,
}

@vertex
fn vertex(@location(0) position: vec3<f32>) -> SunVaryings {
    let forward = sun.direction.xyz;
    // The orbit is tilted, so the sun never reaches the zenith and Y is a safe up.
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), forward));
    let up = cross(forward, right);
    let direction = forward + (position.x * right + position.y * up) * (sun.direction.w * EXTENT);
    var out: SunVaryings;
    // At infinity, like the moon and the stars: depth 0 in reverse Z.
    out.position = view.clip_from_world * vec4<f32>(direction, 0.0);
    out.disk = position.xy * EXTENT;
    return out;
}

@fragment
fn fragment(in: SunVaryings) -> @location(0) vec4<f32> {
    let r2 = dot(in.disk, in.disk);
    if r2 >= 1.0 {
        discard;
    }
    // Limb darkening, and a soft rim against aliasing.
    let mu = sqrt(1.0 - r2);
    let limb = 1.0 - 0.6 * (1.0 - mu);
    let edge = 1.0 - smoothstep(0.92, 1.0, sqrt(r2));
    // Reddened near the horizon, zero below it. Fragment stage only: the atmosphere
    // buffer is not bound for vertex shaders.
    let color = sun.color.rgb * atmosphere_transmittance(
        view.world_position.xyz,
        sun.direction.xyz,
        2.0 * sun.direction.w,
    );
    // Additive: premultiplied color with zero alpha.
    return vec4<f32>(color * (limb * edge), 0.0);
}
