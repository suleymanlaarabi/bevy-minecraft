// Shared by terrain, explicit models and their shadow passes. Timelines are
// immutable; advancing an animation never uploads assets or remeshes a section.
// Bevy's prepasses (including shadows) bind the same clock at slot 1; the
// forward view binds it at slot 11. Keep animation frames synchronized in both.
import bevy_render::globals::Globals;
@if(PREPASS_PIPELINE)
@group(0) @binding(1) var<uniform> globals: Globals;
@if(!PREPASS_PIPELINE)
@group(0) @binding(11) var<uniform> globals: Globals;

@group(constants::MATERIAL_BIND_GROUP) @binding(10)
var<storage, read> surface_animations: array<vec4<u32>>;

struct SurfaceFrame {
    current: u32,
    next: u32,
    blend: f32,
}

fn surface_frame(layer: u32) -> SurfaceFrame {
    // Static surfaces never read the timeline buffer. Bit 11 is reserved by the
    // importer; physical texture arrays have at most 2048 layers.
    let base = layer & ~2048u;
    var result = SurfaceFrame(base, base, 0.0);
    if (layer & 2048u) == 0u || base >= arrayLength(&surface_animations) { return result; }
    let descriptor = surface_animations[base];
    if descriptor.y == 0u { return result; }
    let tick = (globals.time * 20.0) % f32(descriptor.z);
    var low = 0u;
    var high = descriptor.y;
    // Binary search supports nonuniform frame durations and reordered sequences.
    while low < high {
        let middle = (low + high) >> 1u;
        if tick < f32(surface_animations[descriptor.x + middle].y) {
            high = middle;
        } else {
            low = middle + 1u;
        }
    }
    let index = min(low, descriptor.y - 1u);
    let entry = surface_animations[descriptor.x + index];
    result.current = entry.x;
    result.next = surface_animations[descriptor.x + ((index + 1u) % descriptor.y)].x;
    if descriptor.w != 0u {
        var start = 0.0;
        if index > 0u { start = f32(surface_animations[descriptor.x + index - 1u].y); }
        result.blend = (tick - start) / (f32(entry.y) - start);
    }
    return result;
}
