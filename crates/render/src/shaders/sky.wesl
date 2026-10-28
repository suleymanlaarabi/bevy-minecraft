// Every sky and environment pass of `SkyMaterial`. Each pass is a pair of entry
// points selected at pipeline specialization; all share one material bind group.
import super::sky_color::{sample_sky, horizon_haze};
import super::block_lighting::celestial_irradiance;
import super::voxel_fog::apply_voxel_fog;
import bevy_pbr::render::{
    forward_io::FragmentOutput,
    mesh_functions,
    mesh_view_bindings as view_bindings,
    view_transformations::position_world_to_clip,
};

struct SkySettings {
    // Sun and moon: unit vector toward the body in XYZ, angular radius of the disk in W.
    direction: vec4<f32>,
    // Sun: output-space color of the disk's center in RGB.
    // Moon: output-space color in RGB, opacity in A.
    color: vec4<f32>,
    // Moon: lit fraction of the disk in X, +1 waxing or -1 waning in Y.
    // Stars: brightness in X, from 0 by day to 1 at night.
    // Clouds: horizontal distances where the dithered fade starts and ends in XY.
    params: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var<uniform> sky: SkySettings;

// ---- Background: the sky gradient behind everything ----

struct BackgroundVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) direction: vec3<f32>,
}

@vertex
fn background_vertex(@location(0) position: vec3<f32>) -> BackgroundVaryings {
    var out: BackgroundVaryings;
    out.position = vec4<f32>(position.xy, 0.0, 1.0);
    let world = view_bindings::view.world_from_clip * out.position;
    out.direction = world.xyz - view_bindings::view.world_position.xyz * world.w;
    return out;
}

@fragment
fn background_fragment(in: BackgroundVaryings) -> @location(0) vec4<f32> {
    return vec4<f32>(sample_sky(normalize(in.direction)), 1.0);
}

// ---- Stars: point stars on a sphere at infinity ----

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
fn stars_vertex(in: StarVertex) -> StarVaryings {
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let direction = normalize((world_from_local * vec4<f32>(in.position, 0.0)).xyz);
    var out: StarVaryings;
    // A direction (w = 0) projects to the far plane at infinity: depth 0 in reverse Z,
    // so any terrain or cloud in front hides the star.
    out.position = view_bindings::view.clip_from_world * vec4<f32>(direction, 0.0);
    // Extinction near the horizon, where the air is thickest.
    let horizon = smoothstep(-0.02, 0.3, direction.y);
    out.corner = in.corner;
    out.color = vec4<f32>(in.color.rgb * (horizon * sky.params.x), in.color.a);
    return out;
}

@fragment
fn stars_fragment(in: StarVaryings) -> @location(0) vec4<f32> {
    let falloff = max(1.0 - dot(in.corner, in.corner), 0.0);
    let seed = in.color.a;
    let twinkle = 0.7 + 0.3 * sin(view_bindings::globals.time * (1.5 + 3.0 * seed) + 40.0 * seed);
    // Additive: premultiplied color with zero alpha.
    return vec4<f32>(in.color.rgb * (falloff * falloff * twinkle), 0.0);
}

// ---- Sun and moon: camera-facing disks at infinity ----

struct DiskVaryings {
    @builtin(position) position: vec4<f32>,
    // Disk coordinates: the body is the unit circle.
    @location(0) disk: vec2<f32>,
}

// A quad around `sky.direction`, reaching `extent` disk radii from its center.
fn disk_vertex(position: vec3<f32>, extent: f32) -> DiskVaryings {
    let forward = sky.direction.xyz;
    // The orbits are tilted, so neither body reaches the zenith and Y is a safe up.
    // Face the observer inside the sky, so back-face culling keeps the disk.
    let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, forward);
    let direction = forward + (position.x * right + position.y * up) * (sky.direction.w * extent);
    var out: DiskVaryings;
    // At infinity, like the stars: depth 0 in reverse Z.
    out.position = view_bindings::view.clip_from_world * vec4<f32>(direction, 0.0);
    out.disk = position.xy * extent;
    return out;
}

// Keep the sun's glow visible even when bloom is disabled.
const SUN_EXTENT: f32 = 3.5;

@vertex
fn sun_vertex(@location(0) position: vec3<f32>) -> DiskVaryings {
    return disk_vertex(position, SUN_EXTENT);
}

@fragment
fn sun_fragment(in: DiskVaryings) -> @location(0) vec4<f32> {
    let r = length(in.disk);
    let edge_width = max(fwidth(r), 0.002);
    if r >= SUN_EXTENT {
        discard;
    }
    // A bright ivory center, a golden limb, and a smooth edge at any resolution.
    let mu = sqrt(max(1.0 - r * r, 0.0));
    let limb = 1.0 - 0.6 * (1.0 - mu);
    let edge = 1.0 - smoothstep(1.0 - edge_width, 1.0 + edge_width, r);
    let disk = mix(vec3<f32>(1.0, 0.67, 0.30), vec3<f32>(1.0, 0.98, 0.90), mu) * (limb * edge);
    let outside = max(r - 1.0, 0.0);
    let halo = (0.018 * exp(-3.0 * outside) + 0.003 * exp(-outside * outside))
        * (1.0 - smoothstep(2.0, SUN_EXTENT, r));
    let color = sky.color.rgb;
    // Additive: premultiplied color with zero alpha.
    return vec4<f32>(color * (disk + vec3<f32>(1.0, 0.57, 0.20) * halo), 0.0);
}

// The moon's quad reaches past the disk to leave room for the halo.
const MOON_EXTENT: f32 = 3.0;

@vertex
fn moon_vertex(@location(0) position: vec3<f32>) -> DiskVaryings {
    return disk_vertex(position, MOON_EXTENT);
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

// A dark bowl with a bright raised rim; all detail stays fixed on the surface.
fn crater(p: vec2<f32>, center: vec2<f32>, radius: f32) -> f32 {
    let distance = length(p - center) / radius;
    let bowl = 1.0 - smoothstep(0.35, 0.85, distance);
    let rim = smoothstep(0.75, 0.95, distance) * (1.0 - smoothstep(1.0, 1.18, distance));
    return 0.13 * rim - 0.16 * bowl;
}

// Dark maria and distinct craters on a bright highland surface.
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
    let relief = crater(p, vec2<f32>(0.38, -0.42), 0.19)
        + crater(p, vec2<f32>(-0.28, 0.32), 0.13)
        + crater(p, vec2<f32>(0.55, 0.24), 0.10)
        + crater(p, vec2<f32>(-0.48, -0.30), 0.15);
    return 0.9 - 0.38 * smoothstep(0.48, 0.66, maria)
        - 0.1 * smoothstep(0.7, 0.95, craters) + relief;
}

@fragment
fn moon_fragment(in: DiskVaryings) -> @location(0) vec4<f32> {
    let r2 = dot(in.disk, in.disk);
    let r = sqrt(r2);
    let edge_width = max(fwidth(r), 0.002);
    let lit_fraction = sky.params.x;
    var color = vec3<f32>(0.0);
    if r2 < 1.0 {
        // Sphere normal, Z toward the viewer.
        let normal = vec3<f32>(in.disk, sqrt(1.0 - r2));
        // The sun swings from behind the moon (new) to behind the viewer (full).
        let cos_phase = 2.0 * lit_fraction - 1.0;
        let sun = vec3<f32>(sky.params.y * sqrt(max(1.0 - cos_phase * cos_phase, 0.0)), 0.0, cos_phase);
        let lit = smoothstep(-0.04, 0.06, dot(normal, sun));
        // Faint earthshine keeps the dark side barely visible.
        let light = lit * (0.65 + 0.35 * normal.z) + 0.025;
        let edge = 1.0 - smoothstep(1.0 - edge_width, 1.0, r);
        color = vec3<f32>(moon_albedo(in.disk) * light * edge);
    }
    // Scattered glow around the disk, stronger when more of it is lit.
    let halo = 0.04 * lit_fraction * exp(-3.5 * max(r - 1.0, 0.0))
        * smoothstep(0.8, 1.1, r) * (1.0 - smoothstep(2.0, MOON_EXTENT, r));
    color += vec3<f32>(0.65, 0.78, 1.0) * halo;
    // Additive: premultiplied color with zero alpha.
    return vec4<f32>(color * sky.color.rgb * sky.color.a, 0.0);
}

// ---- Clouds: opaque, flat-shaded boxes lit like terrain ----

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
fn clouds_vertex(in: CloudVertex) -> CloudVaryings {
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
fn clouds_fragment(in: CloudVaryings) -> FragmentOutput {
    let fade = sky.params;
    // Opaque clouds dissolve toward the edge of the layer.
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
        mix(
            vec3<f32>(0.96) * face * irradiance * view_bindings::view.exposure,
            sample_sky(normalize(in.world_position.xyz - view_bindings::view.world_position.xyz)),
            0.65 * smoothstep(0.0, fade.y, distance),
        ),
        1.0,
    );
    return out;
}

// ---- Horizon: lower-sky haze and the opaque immersion dome ----

struct HorizonVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
}

@vertex
fn horizon_vertex(
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
) -> HorizonVaryings {
    let world_from_local = mesh_functions::get_world_from_local(instance_index);
    var out: HorizonVaryings;
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
    return out;
}

// Fade the lower background into the sky gradient at the same horizon color.
// Keep the rim transparent
// without writing a depth edge across the sky or hiding celestial disks.
@fragment
fn horizon_fragment(in: HorizonVaryings) -> @location(0) vec4<f32> {
@if(DISTANCE_FOG) {
    if view_bindings::fog.directional_light_color.a > 0.0 {
        let direction = normalize(in.world_position.xyz - view_bindings::view.world_position.xyz);
        return horizon_haze(direction);
    }
}
    // The liquid dome is fully opaque and keeps the original immersion color.
    return apply_voxel_fog(vec4<f32>(0.0, 0.0, 0.0, 1.0), in.world_position.xyz);
}
