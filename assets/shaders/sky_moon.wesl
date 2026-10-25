import bevy_pbr::render::mesh_view_bindings::view;

struct MoonSettings {
    // Unit vector toward the moon in XYZ, angular radius of the disk in W.
    direction: vec4<f32>,
    // Output-space color in RGB, opacity in A.
    color: vec4<f32>,
    // Lit fraction of the disk in X, +1 waxing or -1 waning in Y.
    phase: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> moon: MoonSettings;

// The quad reaches past the disk to leave room for the halo.
const HALO_EXTENT: f32 = 3.0;

struct MoonVaryings {
    @builtin(position) position: vec4<f32>,
    // Disk coordinates: the moon is the unit circle.
    @location(0) disk: vec2<f32>,
}

@vertex
fn vertex(@location(0) position: vec3<f32>) -> MoonVaryings {
    let forward = moon.direction.xyz;
    // The moon never reaches the zenith (its orbit is tilted), so Y is a safe up.
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), forward));
    let up = cross(forward, right);
    let extent = moon.direction.w * HALO_EXTENT;
    let direction = forward + (position.x * right + position.y * up) * extent;
    var out: MoonVaryings;
    // At infinity, like the stars: depth 0 in reverse Z.
    out.position = view.clip_from_world * vec4<f32>(direction, 0.0);
    out.disk = position.xy * HALO_EXTENT;
    return out;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(cell), hash(cell + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(cell + vec2<f32>(0.0, 1.0)), hash(cell + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

// Dark maria and small craters on a bright highland surface.
fn moon_albedo(p: vec2<f32>) -> f32 {
    var maria = 0.0;
    var amplitude = 0.5;
    var q = p * 2.2 + vec2<f32>(3.1, 1.7);
    for (var i = 0; i < 4; i += 1) {
        maria += amplitude * value_noise(q);
        q *= 2.03;
        amplitude *= 0.5;
    }
    let craters = value_noise(p * 14.0);
    return 0.9 - 0.38 * smoothstep(0.48, 0.66, maria) - 0.1 * smoothstep(0.7, 0.95, craters);
}

@fragment
fn fragment(in: MoonVaryings) -> @location(0) vec4<f32> {
    let r2 = dot(in.disk, in.disk);
    let r = sqrt(r2);
    let lit_fraction = moon.phase.x;
    var color = vec3<f32>(0.0);
    if r2 < 1.0 {
        // Sphere normal, Z toward the viewer.
        let normal = vec3<f32>(in.disk, sqrt(1.0 - r2));
        // The sun swings from behind the moon (new) to behind the viewer (full).
        let cos_phase = 2.0 * lit_fraction - 1.0;
        let sun = vec3<f32>(moon.phase.y * sqrt(max(1.0 - cos_phase * cos_phase, 0.0)), 0.0, cos_phase);
        let lit = smoothstep(-0.04, 0.06, dot(normal, sun));
        // Faint earthshine keeps the dark side barely visible.
        let light = lit + 0.025;
        let edge = 1.0 - smoothstep(0.96, 1.0, r);
        color = vec3<f32>(moon_albedo(in.disk) * light * edge);
    }
    // Scattered glow around the disk, stronger when more of it is lit.
    let halo = 0.06 * lit_fraction * exp(-3.5 * max(r - 1.0, 0.0)) * (1.0 - step(r, 1.0) * 0.6);
    color += vec3<f32>(halo);
    // Additive: premultiplied color with zero alpha.
    return vec4<f32>(color * moon.color.rgb * moon.color.a, 0.0);
}
