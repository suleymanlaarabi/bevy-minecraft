import bevy_pbr::atmosphere::{
    bindings::{settings, view, aerial_view_lut, atmosphere_lut_sampler},
    functions::{get_view_position, direction_world_to_atmosphere, sample_sky_view_lut},
};

@group(1) @binding(0)
var<storage, read_write> snapshot: array<vec4<f32>>;

@if(FILTERED_FOG) {
@group(1) @binding(1) var sky_output: texture_storage_2d<rgba32float, write>;
@group(1) @binding(2) var aerial_output: texture_storage_3d<rgba32float, write>;
}

// Two metadata vectors plus a 128x64 sky and a 32x32x32 aerial LUT.
// Filterable devices store the LUTs in textures; other devices append them here.
const SKY_WIDTH: u32 = 128u;
const SKY_HEIGHT: u32 = 64u;
const SKY_OFFSET: u32 = 2u;
const AERIAL_OFFSET: u32 = SKY_OFFSET + SKY_WIDTH * SKY_HEIGHT;
const AERIAL_SIZE: u32 = 32u;
const PI: f32 = 3.141592653589793;

@compute
@workgroup_size(64)
fn main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    if index == 0u {
        // Bevy can still be compiling its LUT pipelines during the first frames.
        // Their textures stay zero until all atmosphere pipelines are ready.
        let zenith = sample_sky_view_lut(length(get_view_position()), direction_world_to_atmosphere(vec3<f32>(0.0, 1.0, 0.0)));
        let ready = select(0.0, 1.0, any(zenith > vec3<f32>(0.0)));
        snapshot[0] = vec4<f32>(ready, f32(SKY_WIDTH), f32(SKY_HEIGHT), settings.aerial_view_lut_max_distance);
        var rgb_transmittance = 0.0;
@if(RGB_TRANSMITTANCE) {
        rgb_transmittance = 1.0;
}
        snapshot[1] = vec4<f32>(vec3<f32>(f32(AERIAL_SIZE)), rgb_transmittance);
    }
    if index < SKY_WIDTH * SKY_HEIGHT {
        let uv = (vec2<f32>(f32(index % SKY_WIDTH), f32(index / SKY_WIDTH)) + 0.5)
            / vec2<f32>(f32(SKY_WIDTH), f32(SKY_HEIGHT));
        let azimuth = (uv.x - 0.5) * (2.0 * PI);
        // Concentrate samples near eye level, where the horizon transition lies.
        let latitude = uv.y * 2.0 - 1.0;
        let elevation = sign(latitude) * latitude * latitude * (0.5 * PI);
        let direction = vec3<f32>(sin(azimuth) * cos(elevation), sin(elevation), -cos(azimuth) * cos(elevation));
        let sky = sample_sky_view_lut(length(get_view_position()), direction_world_to_atmosphere(direction));
@if(FILTERED_FOG) {
        textureStore(sky_output, vec2<i32>(i32(index % SKY_WIDTH), i32(index / SKY_WIDTH)), vec4<f32>(sky * view.exposure, 1.0));
} @else {
        snapshot[SKY_OFFSET + index] = vec4<f32>(sky * view.exposure, 1.0);
}
    }
    if index < AERIAL_SIZE * AERIAL_SIZE * AERIAL_SIZE {
        let position = vec3<u32>(index % AERIAL_SIZE, (index / AERIAL_SIZE) % AERIAL_SIZE, index / (AERIAL_SIZE * AERIAL_SIZE));
        // The camera keeps Bevy's default 32-cubed LUT. Sampling at texel centers
        // preserves it exactly, and still supports another source resolution.
        let uvw = (vec3<f32>(position) + 0.5) / f32(AERIAL_SIZE);
        let aerial = textureSampleLevel(
            aerial_view_lut, atmosphere_lut_sampler, uvw, 0.0,
        );
@if(FILTERED_FOG) {
        textureStore(aerial_output, vec3<i32>(position), aerial);
} @else {
        snapshot[AERIAL_OFFSET + index] = aerial;
}
    }
}
