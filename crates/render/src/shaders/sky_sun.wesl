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

// Keep the glow visible even when bloom is disabled.
const EXTENT: f32 = 3.5;

struct SunVaryings {
    @builtin(position) position: vec4<f32>,
    // Disk coordinates: the sun is the unit circle.
    @location(0) disk: vec2<f32>,
}

@vertex
fn vertex(@location(0) position: vec3<f32>) -> SunVaryings {
    let forward = sun.direction.xyz;
    // The orbit is tilted, so the sun never reaches the zenith and Y is a safe up.
    // Face the observer inside the sky, so back-face culling keeps the disk.
    let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, forward);
    let direction = forward + (position.x * right + position.y * up) * (sun.direction.w * EXTENT);
    var out: SunVaryings;
    // At infinity, like the moon and the stars: depth 0 in reverse Z.
    out.position = view.clip_from_world * vec4<f32>(direction, 0.0);
    out.disk = position.xy * EXTENT;
    return out;
}

@fragment
fn fragment(in: SunVaryings) -> @location(0) vec4<f32> {
    let r = length(in.disk);
    let edge_width = max(fwidth(r), 0.002);
    if r >= EXTENT {
        discard;
    }
    // A bright ivory center, a golden limb, and a smooth edge at any resolution.
    let mu = sqrt(max(1.0 - r * r, 0.0));
    let limb = 1.0 - 0.6 * (1.0 - mu);
    let edge = 1.0 - smoothstep(1.0 - edge_width, 1.0 + edge_width, r);
    let disk = mix(vec3<f32>(1.0, 0.67, 0.30), vec3<f32>(1.0, 0.98, 0.90), mu) * (limb * edge);
    let outside = max(r - 1.0, 0.0);
    let halo = (0.018 * exp(-3.0 * outside) + 0.003 * exp(-outside * outside))
        * (1.0 - smoothstep(2.0, EXTENT, r));
    // Reddened near the horizon, zero below it. Fragment stage only: the atmosphere
    // buffer is not bound for vertex shaders.
    let color = sun.color.rgb * atmosphere_transmittance(
        view.world_position.xyz,
        sun.direction.xyz,
        2.0 * sun.direction.w,
    );
    // Additive: premultiplied color with zero alpha.
    return vec4<f32>(color * (disk + vec3<f32>(1.0, 0.57, 0.20) * halo), 0.0);
}
