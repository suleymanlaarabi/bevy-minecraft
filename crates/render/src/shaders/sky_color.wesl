// Display-space palette shared by the sky, distance fog and water reflection
// (`fog::upload_palette`). Entries 4 to 9 hold the per-frame sun, moon and star
// inputs, so the day cycle never modifies a material.
@group(constants::MATERIAL_BIND_GROUP) @binding(11)
var<storage, read> sky_palette: array<vec4<f32>>;

const SUN_DIRECTION: u32 = 4u;
const SUN_COLOR: u32 = 5u;
const MOON_DIRECTION: u32 = 6u;
const MOON_COLOR: u32 = 7u;
const MOON_PARAMS: u32 = 8u;
const STAR_PARAMS: u32 = 9u;

fn palette_entry(index: u32) -> vec4<f32> {
    return sky_palette[index];
}

fn sample_sky(direction: vec3<f32>) -> vec3<f32> {
    let height = smoothstep(0.0, 0.65, max(direction.y, 0.0));
    let base = mix(sky_palette[1].rgb, sky_palette[0].rgb, height);
    let sunset = sky_palette[3].rgb;
    if all(sunset == vec3<f32>(0.0)) { return base; }
    let sun = sky_palette[2].xyz;
    let horizontal = direction.xz / max(length(direction.xz), 1e-4);
    let sun_horizontal = sun.xz / max(length(sun.xz), 1e-4);
    let facing = max(dot(horizontal, sun_horizontal), 0.0);
    let glow = facing * facing * facing * facing
        * (1.0 - smoothstep(0.0, 0.4, abs(direction.y)));
    return base + sunset * glow;
}

fn horizon_haze(direction: vec3<f32>) -> vec4<f32> {
    let horizon = vec3<f32>(direction.x, 0.0, direction.z);
    return vec4<f32>(sample_sky(horizon), 1.0 - smoothstep(0.0, 0.052335956, direction.y));
}
