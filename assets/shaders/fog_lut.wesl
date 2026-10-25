import bevy_pbr::render::mesh_view_bindings as view_bindings;
import bevy_pbr::atmosphere::{
    functions::clamp_to_surface,
    bruneton_functions::transmittance_lut_r_mu_to_uv,
};

// Written once per frame by prepare_fog.wesl, before the opaque main pass.
@group(constants::MATERIAL_BIND_GROUP) @binding(11)
var<storage, read> snapshot: array<vec4<f32>>;

@if(FILTERED_FOG) {
@group(constants::MATERIAL_BIND_GROUP) @binding(12) var fog_sky: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(13) var fog_sky_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(14) var fog_aerial: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(15) var fog_aerial_sampler: sampler;
}

const SKY_WIDTH: i32 = 128;
const SKY_HEIGHT: i32 = 64;
const SKY_OFFSET: u32 = 2u;
const AERIAL_OFFSET: u32 = SKY_OFFSET + 128u * 64u;
const AERIAL_SIZE: i32 = 32;
const PI: f32 = 3.141592653589793;

fn has_fog_lut() -> bool {
    return snapshot[0].x > 0.0;
}

fn sky_texel(position: vec2<i32>) -> vec3<f32> {
    let x = ((position.x % SKY_WIDTH) + SKY_WIDTH) % SKY_WIDTH;
    let y = clamp(position.y, 0, SKY_HEIGHT - 1);
    return snapshot[SKY_OFFSET + u32(x + y * SKY_WIDTH)].rgb;
}

fn sample_fog_sky(direction: vec3<f32>) -> vec3<f32> {
    let elevation = asin(clamp(direction.y, -1.0, 1.0));
    let latitude = sign(elevation) * sqrt(abs(elevation) / (0.5 * PI));
    let horizontal = select(vec2<f32>(0.0, -1.0), direction.xz, length(direction.xz) > 1e-5);
    let uv = vec2<f32>(atan2(horizontal.x, -horizontal.y) / (2.0 * PI) + 0.5, latitude * 0.5 + 0.5);
@if(FILTERED_FOG) {
    return textureSampleLevel(fog_sky, fog_sky_sampler, uv, 0.0).rgb;
} @else {
    let pixel = uv * vec2<f32>(f32(SKY_WIDTH), f32(SKY_HEIGHT)) - 0.5;
    let cell = vec2<i32>(floor(pixel));
    let weight = fract(pixel);
    return mix(
        mix(sky_texel(cell), sky_texel(cell + vec2<i32>(1, 0)), weight.x),
        mix(sky_texel(cell + vec2<i32>(0, 1)), sky_texel(cell + vec2<i32>(1, 1)), weight.x),
        weight.y,
    );
}

}

// The lower sky fades into the upper sky over three degrees. This also supplies the
// skirt's alpha, leaving the real upper sky, sun, moon and stars visible.
fn horizon_haze(direction: vec3<f32>) -> vec4<f32> {
    let reach = length(direction.xz);
    let horizontal = select(vec2<f32>(0.0, -1.0), direction.xz / max(reach, 1e-5), reach > 1e-5);
    var color = vec3<f32>(0.5);
@if(DISTANCE_FOG) {
    color = view_bindings::fog.base_color.rgb;
}
    if has_fog_lut() {
        color = sample_fog_sky(vec3<f32>(horizontal.x, 0.0, horizontal.y));
    }
    let opacity = 1.0 - smoothstep(0.0, 0.052335956, direction.y);
    return vec4<f32>(color, opacity);
}

fn natural_fog_color(direction: vec3<f32>) -> vec3<f32> {
    let haze = horizon_haze(direction);
    if !has_fog_lut() || haze.a >= 1.0 {
        return haze.rgb;
    }
    // Match the actual sky behind the skirt, including the smooth angular blend.
    return mix(sample_fog_sky(direction), haze.rgb, haze.a);
}

fn aerial_texel(position: vec3<i32>) -> vec3<f32> {
    let p = vec3<u32>(clamp(position, vec3<i32>(0), vec3<i32>(AERIAL_SIZE - 1)));
    return snapshot[AERIAL_OFFSET + p.x + 32u * (p.y + 32u * p.z)].rgb;
}

// Trilinear interpolation in Bevy's logarithmic aerial-perspective LUT.
fn aerial_slice(cell: vec3<i32>, weight: vec2<f32>) -> vec3<f32> {
    return mix(
        mix(aerial_texel(cell), aerial_texel(cell + vec3<i32>(1, 0, 0)), weight.x),
        mix(aerial_texel(cell + vec3<i32>(0, 1, 0)), aerial_texel(cell + vec3<i32>(1, 1, 0)), weight.x),
        weight.y,
    );
}

fn aerial_inscattering(world_position: vec3<f32>, distance: f32) -> vec3<f32> {
    let clip = view_bindings::view.clip_from_world * vec4<f32>(world_position, 1.0);
    let uv = clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5;
    let max_distance = snapshot[0].w;
    let uvw = vec3<f32>(uv, clamp(distance / max_distance - 0.5 / f32(AERIAL_SIZE), 0.0, 1.0));
    var logarithm: vec3<f32>;
@if(FILTERED_FOG) {
    logarithm = textureSampleLevel(fog_aerial, fog_aerial_sampler, uvw, 0.0).rgb;
} @else {
    let pixel = uvw * f32(AERIAL_SIZE) - 0.5;
    let cell = vec3<i32>(floor(pixel));
    let weight = fract(pixel);
    logarithm = mix(aerial_slice(cell, weight.xy), aerial_slice(cell + vec3<i32>(0, 0, 1), weight.xy), weight.z);

}
    let fade = clamp(distance * f32(AERIAL_SIZE) / max_distance, 0.0, 1.0);
    return exp(logarithm) * fade * view_bindings::view.exposure;
}

@if(ATMOSPHERE)
fn transmittance_to_space(position: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    let r = length(position);
    let uv = transmittance_lut_r_mu_to_uv(view_bindings::atmosphere, r, dot(position / r, direction));
    return textureSampleLevel(
        view_bindings::atmosphere_transmittance_texture,
        view_bindings::atmosphere_transmittance_sampler, uv, 0.0,
    ).rgb;
}

// Opaque fragments are subsequently transformed as C * T + S by Bevy. Encode the
// fog target as (target - S) / T, so its final color matches transparent water and
// the horizon skirt. Signed HDR values are intentional and must not be clamped.
fn fog_before_atmosphere(fog_target: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
@if(ATMOSPHERE) {
    let atmosphere = view_bindings::atmosphere;
    let camera = view_bindings::view.world_position.xyz;
    let origin = clamp_to_surface(atmosphere, (atmosphere.world_to_atmosphere * vec4<f32>(camera, 1.0)).xyz);
    let offset = (atmosphere.world_to_atmosphere * vec4<f32>(world_position - camera, 0.0)).xyz;
    let distance = length(offset);
    let direction = offset / max(distance, 1e-5);
    // Choose the common outer boundary reached by extending this segment. For a
    // ground-facing ray, use its reverse so neither lookup crosses the planet.
    let closest = origin + direction * max(-dot(origin, direction), 0.0);
    let crosses_ground = dot(origin, direction) < 0.0 && length(closest) <= atmosphere.inner_radius;
    let to_space = select(direction, -direction, crosses_ground);
    let start = transmittance_to_space(origin, to_space);
    let end = transmittance_to_space(origin + offset, to_space);
    var transmittance = min(select(start / max(end, vec3<f32>(1e-6)), end / max(start, vec3<f32>(1e-6)), crosses_ground), vec3<f32>(1.0));
    if snapshot[1].w < 0.5 {
        transmittance = vec3<f32>(dot(transmittance, vec3<f32>(1.0 / 3.0)));
    }
    return (fog_target - aerial_inscattering(world_position, distance)) / max(transmittance, vec3<f32>(1e-4));
} @else {
    return fog_target;
}
}
