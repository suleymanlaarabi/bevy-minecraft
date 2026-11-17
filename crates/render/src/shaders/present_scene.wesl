// Bloom composite, tonemapping and FXAA in one full-screen pass, written straight
// to the window.
//
// Bevy blends its bloom pyramid into the whole scene, tonemaps the scene into a
// second scene-format texture, then FXAA reads it back: two full-screen writes and
// reads, and tonemapped colors requantized to the scene's 11/11/10-bit floats.
// Here the bloom (Bevy's final tent upsample and blend, `bloom.rs`) is added as the
// scene is read. FXAA (Bevy's 3.11 port, same thresholds) reads the scene itself:
// edge detection uses a Reinhard-compressed luma, a perceptual proxy for the
// tonemapped one, and only output colors are tonemapped exactly: the center pixel,
// or on edges the two texels Bevy's bilinear tap would blend. The bloom of the
// center stands for its neighbors': the pyramid's first level is several pixels
// wide and smooth.
import bevy_render::{
    view::View,
    maths::powsafe,
};
import bevy_core_pipeline::{
    fullscreen_vertex_shader::FullscreenVertexOutput,
    tonemapping::{tone_mapping, screen_space_dither},
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var scene_sampler: sampler;
// The bloom pyramid's first level, and its blend factor and scale.
@if(BLOOM)
@group(0) @binding(5) var bloom_texture: texture_2d<f32>;
@if(BLOOM)
@group(0) @binding(6) var<uniform> bloom_params: vec4<f32>;

@if(EDGE_THRESH_MIN_LOW)
const EDGE_THRESHOLD_MIN: f32 = 0.0833;
@if(EDGE_THRESH_MIN_MEDIUM)
const EDGE_THRESHOLD_MIN: f32 = 0.0625;
@if(EDGE_THRESH_MIN_HIGH)
const EDGE_THRESHOLD_MIN: f32 = 0.0312;
@if(EDGE_THRESH_MIN_ULTRA)
const EDGE_THRESHOLD_MIN: f32 = 0.0156;
@if(EDGE_THRESH_MIN_EXTREME)
const EDGE_THRESHOLD_MIN: f32 = 0.0078;

@if(EDGE_THRESH_LOW)
const EDGE_THRESHOLD_MAX: f32 = 0.250;
@if(EDGE_THRESH_MEDIUM)
const EDGE_THRESHOLD_MAX: f32 = 0.166;
@if(EDGE_THRESH_HIGH)
const EDGE_THRESHOLD_MAX: f32 = 0.125;
@if(EDGE_THRESH_ULTRA)
const EDGE_THRESHOLD_MAX: f32 = 0.063;
@if(EDGE_THRESH_EXTREME)
const EDGE_THRESHOLD_MAX: f32 = 0.031;

const ITERATIONS: i32 = 12;
const SUBPIXEL_QUALITY: f32 = 0.75;

fn quality(q: i32) -> f32 {
    switch (q) {
        default: { return 1.0; }
        case 5: { return 1.5; }
        case 6, 7, 8, 9: { return 2.0; }
        case 10: { return 4.0; }
        case 11: { return 8.0; }
    }
}

fn sample_scene(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(scene, scene_sampler, uv, 0.0).rgb;
}

// Bevy's final bloom upsample: a 3x3 tent over the pyramid's first level.
fn glow_at(uv: vec2<f32>) -> vec3<f32> {
@if(BLOOM) {
    let size = bloom_params.yz / vec2<f32>(textureDimensions(bloom_texture));
    let x = size.x;
    let y = size.y;
    let a = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x - x, uv.y + y), 0.0).rgb;
    let b = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x, uv.y + y), 0.0).rgb;
    let c = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x + x, uv.y + y), 0.0).rgb;
    let d = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x - x, uv.y), 0.0).rgb;
    let e = textureSampleLevel(bloom_texture, scene_sampler, uv, 0.0).rgb;
    let f = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x + x, uv.y), 0.0).rgb;
    let g = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x - x, uv.y - y), 0.0).rgb;
    let h = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x, uv.y - y), 0.0).rgb;
    let i = textureSampleLevel(bloom_texture, scene_sampler, vec2<f32>(uv.x + x, uv.y - y), 0.0).rgb;
    return e * 0.25 + (b + d + f + h) * 0.125 + (a + c + g + i) * 0.0625;
} @else {
    return vec3<f32>(0.0);
}
}

// Bevy's bloom blend: energy-conserving or additive, by the blend constant.
fn composite(scene: vec3<f32>, glow: vec3<f32>) -> vec3<f32> {
@if(BLOOM_ADDITIVE) {
    return scene + glow * bloom_params.x;
} @elif(BLOOM) {
    return mix(scene, glow, bloom_params.x);
} @else {
    return scene;
}
}

// FXAA's gamma-2 luma of a tonemapped color, with Reinhard standing in for the
// tonemapping curve: thresholds see perceptual contrast, not raw scene radiance.
fn luma(rgb: vec3<f32>) -> f32 {
    let linear = dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
    return sqrt(linear / (1.0 + linear));
}

fn tonemap(rgb: vec3<f32>) -> vec3<f32> {
    return tone_mapping(vec4<f32>(rgb, 1.0), view.color_grading).rgb;
}

fn finish(rgb: vec3<f32>, frag_coord: vec2<f32>) -> vec4<f32> {
    var output = rgb;
@if(DEBAND_DITHER) {
    output = powsafe(output, 1.0 / 2.2);
    output = output + screen_space_dither(frag_coord);
    // The window format is sRGB: it encodes linear values.
    output = powsafe(output, 2.2);
}
    return vec4<f32>(output, 1.0);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let inverseScreenSize = 1.0 / vec2<f32>(textureDimensions(scene));
    let texCoord = in.position.xy * inverseScreenSize;
    let glow = glow_at(texCoord);
    let colorCenter = composite(sample_scene(texCoord), glow);
@if(FXAA) {
    return finish(fxaa(texCoord, inverseScreenSize, colorCenter, glow), in.position.xy);
} @else {
    return finish(tonemap(colorCenter), in.position.xy);
}
}

fn fxaa(
    texCoord: vec2<f32>,
    inverseScreenSize: vec2<f32>,
    colorCenter: vec3<f32>,
    glow: vec3<f32>,
) -> vec3<f32> {
    let lumaCenter = luma(colorCenter);
    let lumaDown = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(0, -1)).rgb, glow));
    let lumaUp = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(0, 1)).rgb, glow));
    let lumaLeft = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(-1, 0)).rgb, glow));
    let lumaRight = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(1, 0)).rgb, glow));

    let lumaMin = min(lumaCenter, min(min(lumaDown, lumaUp), min(lumaLeft, lumaRight)));
    let lumaMax = max(lumaCenter, max(max(lumaDown, lumaUp), max(lumaLeft, lumaRight)));
    let lumaRange = lumaMax - lumaMin;
    if (lumaRange < max(EDGE_THRESHOLD_MIN, lumaMax * EDGE_THRESHOLD_MAX)) {
        return tonemap(colorCenter);
    }

    let lumaDownLeft = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(-1, -1)).rgb, glow));
    let lumaUpRight = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(1, 1)).rgb, glow));
    let lumaUpLeft = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(-1, 1)).rgb, glow));
    let lumaDownRight = luma(composite(textureSampleLevel(scene, scene_sampler, texCoord, 0.0, vec2<i32>(1, -1)).rgb, glow));

    let lumaDownUp = lumaDown + lumaUp;
    let lumaLeftRight = lumaLeft + lumaRight;
    let lumaLeftCorners = lumaDownLeft + lumaUpLeft;
    let lumaDownCorners = lumaDownLeft + lumaDownRight;
    let lumaRightCorners = lumaDownRight + lumaUpRight;
    let lumaUpCorners = lumaUpRight + lumaUpLeft;

    let edgeHorizontal = abs(-2.0 * lumaLeft + lumaLeftCorners)
        + abs(-2.0 * lumaCenter + lumaDownUp) * 2.0
        + abs(-2.0 * lumaRight + lumaRightCorners);
    let edgeVertical = abs(-2.0 * lumaUp + lumaUpCorners)
        + abs(-2.0 * lumaCenter + lumaLeftRight) * 2.0
        + abs(-2.0 * lumaDown + lumaDownCorners);
    let isHorizontal = (edgeHorizontal >= edgeVertical);

    var stepLength = select(inverseScreenSize.x, inverseScreenSize.y, isHorizontal);
    let luma1 = select(lumaLeft, lumaDown, isHorizontal);
    let luma2 = select(lumaRight, lumaUp, isHorizontal);
    let gradient1 = luma1 - lumaCenter;
    let gradient2 = luma2 - lumaCenter;
    let is1Steepest = abs(gradient1) >= abs(gradient2);
    let gradientScaled = 0.25 * max(abs(gradient1), abs(gradient2));

    var lumaLocalAverage = 0.0;
    if (is1Steepest) {
        stepLength = -stepLength;
        lumaLocalAverage = 0.5 * (luma1 + lumaCenter);
    } else {
        lumaLocalAverage = 0.5 * (luma2 + lumaCenter);
    }

    var currentUv = texCoord;
    var offset = vec2<f32>(0.0, 0.0);
    if (isHorizontal) {
        currentUv.y = currentUv.y + stepLength * 0.5;
        offset.x = inverseScreenSize.x;
    } else {
        currentUv.x = currentUv.x + stepLength * 0.5;
        offset.y = inverseScreenSize.y;
    }

    var uv1 = currentUv - offset;
    var uv2 = currentUv + offset;
    var lumaEnd1 = luma(composite(sample_scene(uv1), glow)) - lumaLocalAverage;
    var lumaEnd2 = luma(composite(sample_scene(uv2), glow)) - lumaLocalAverage;
    var reached1 = abs(lumaEnd1) >= gradientScaled;
    var reached2 = abs(lumaEnd2) >= gradientScaled;
    var reachedBoth = reached1 && reached2;
    uv1 = select(uv1 - offset, uv1, reached1);
    uv2 = select(uv2 - offset, uv2, reached2);

    if (!reachedBoth) {
        for (var i: i32 = 2; i < ITERATIONS; i = i + 1) {
            if (!reached1) {
                lumaEnd1 = luma(composite(sample_scene(uv1), glow)) - lumaLocalAverage;
            }
            if (!reached2) {
                lumaEnd2 = luma(composite(sample_scene(uv2), glow)) - lumaLocalAverage;
            }
            reached1 = abs(lumaEnd1) >= gradientScaled;
            reached2 = abs(lumaEnd2) >= gradientScaled;
            reachedBoth = reached1 && reached2;
            if (!reached1) {
                uv1 = uv1 - offset * quality(i);
            }
            if (!reached2) {
                uv2 = uv2 + offset * quality(i);
            }
            if (reachedBoth) {
                break;
            }
        }
    }

    let distance1 = select(texCoord.y - uv1.y, texCoord.x - uv1.x, isHorizontal);
    let distance2 = select(uv2.y - texCoord.y, uv2.x - texCoord.x, isHorizontal);
    let isDirection1 = distance1 < distance2;
    let distanceFinal = min(distance1, distance2);
    let edgeThickness = (distance1 + distance2);
    let isLumaCenterSmaller = lumaCenter < lumaLocalAverage;
    let correctVariation1 = (lumaEnd1 < 0.0) != isLumaCenterSmaller;
    let correctVariation2 = (lumaEnd2 < 0.0) != isLumaCenterSmaller;
    let correctVariation = select(correctVariation2, correctVariation1, isDirection1);
    let pixelOffset = -distanceFinal / edgeThickness + 0.5;
    var finalOffset = select(0.0, pixelOffset, correctVariation);

    let lumaAverage = (1.0 / 12.0) * (2.0 * (lumaDownUp + lumaLeftRight) + lumaLeftCorners + lumaRightCorners);
    let subPixelOffset1 = clamp(abs(lumaAverage - lumaCenter) / lumaRange, 0.0, 1.0);
    let subPixelOffset2 = (-2.0 * subPixelOffset1 + 3.0) * subPixelOffset1 * subPixelOffset1;
    let subPixelOffsetFinal = subPixelOffset2 * subPixelOffset2 * SUBPIXEL_QUALITY;
    finalOffset = max(finalOffset, subPixelOffsetFinal);

    // Bevy's final bilinear tap blends the center with one neighbor across the
    // edge, by `finalOffset`: blend the two tonemapped texels the same way.
    var neighbor = texCoord;
    if (isHorizontal) {
        neighbor.y = neighbor.y + sign(stepLength) * inverseScreenSize.y;
    } else {
        neighbor.x = neighbor.x + sign(stepLength) * inverseScreenSize.x;
    }
    return mix(tonemap(colorCenter), tonemap(composite(sample_scene(neighbor), glow)), finalOffset);
}
