// Scales the tonemapped scene into the window compositor's main texture.
struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(scene, scene_sampler, in.uv, 0.0).rgb, 1.0);
}

// For a compositor that blends in sRGB: its main texture stores encoded values.
@fragment
fn fragment_encoded(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let color = max(textureSampleLevel(scene, scene_sampler, in.uv, 0.0).rgb, vec3<f32>(0.0));
    let low = color * 12.92;
    let high = 1.055 * pow(color, vec3<f32>(1.0 / 2.4)) - 0.055;
    return vec4<f32>(select(high, low, color <= vec3<f32>(0.0031308)), 1.0);
}
