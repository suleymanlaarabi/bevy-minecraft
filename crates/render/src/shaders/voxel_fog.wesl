import super::sky_color::sample_sky;
import bevy_pbr::render::{
    mesh_view_bindings as view_bindings,
    mesh_view_types::{FOG_MODE_EXPONENTIAL, FOG_MODE_EXPONENTIAL_SQUARED, FOG_MODE_LINEAR},
};

// Water shares these coordinates between its fog and sky reflection.
struct FogRay {
    direction: vec3<f32>,
    amount: f32,
}

fn fog_ray(world_position: vec3<f32>) -> FogRay {
    let offset = world_position - view_bindings::view.world_position.xyz;
    let distance = length(offset);
    return FogRay(offset / max(distance, 1e-4), fog_amount(offset, distance));
}

// Shared by terrain, models, water and the horizon skirt so all blend into the same
// distance fog. Clouds reach past it: they fade out on their own.
// Open-air fog samples the sky gradient; liquids keep their own spherical falloff.
fn apply_voxel_fog(color: vec4<f32>, world_position: vec3<f32>) -> vec4<f32> {
    return apply_voxel_fog_ray(color, world_position, fog_ray(world_position));
}

fn fog_amount(offset: vec3<f32>, distance: f32) -> f32 {
@if(DISTANCE_FOG) {
    let fog = view_bindings::fog;
    // In open air, horizontal only: loaded terrain ends on a circle but spans whole
    // columns, so the fog must not swallow the ground below a flying camera. Liquids
    // have no scattering, and their short fog is spherical.
    let open_air = fog.directional_light_color.a > 0.0;
    let fog_distance = select(distance, length(offset.xz), open_air);
    var amount = 0.0;
    if fog.mode == FOG_MODE_LINEAR {
        let progress = clamp((fog_distance - fog.be.x) / max(fog.be.y - fog.be.x, 1e-4), 0.0, 1.0);
        amount = select(progress, progress * progress * (3.0 - 2.0 * progress), open_air);
    } else if fog.mode == FOG_MODE_EXPONENTIAL {
        amount = 1.0 - 1.0 / exp(fog_distance * fog.be.x);
    } else if fog.mode == FOG_MODE_EXPONENTIAL_SQUARED {
        let density = fog_distance * fog.be.x;
        amount = 1.0 - 1.0 / exp(density * density);
    }
    return amount;
} @else {
    return 0.0;
}
}

// Fully fogged opaque fragments contribute no surface color. Keep cutout alpha
// tests, but skip texture/lighting work whose result would be multiplied by zero.
fn fully_fogged(ray: FogRay) -> bool {
@if(DISTANCE_FOG) {
    return ray.amount * view_bindings::fog.base_color.a == 1.0;
} @else {
    return false;
}
}

fn apply_voxel_fog_ray(color: vec4<f32>, world_position: vec3<f32>, ray: FogRay) -> vec4<f32> {
@if(DISTANCE_FOG) {
    let amount = ray.amount;
    if amount <= 0.0 { return color; }
    let fog = view_bindings::fog;
    let open_air = fog.directional_light_color.a > 0.0;
    var fog_color = fog.base_color.rgb;
    let direction = ray.direction;
    if open_air {
        fog_color = sample_sky(direction);
    }
    return vec4<f32>(mix(color.rgb, fog_color, amount * fog.base_color.a), color.a);
} @else {
    return color;
}
}

// Sky color reflected by water at grazing angles, in output space.
fn sky_reflection(fallback: vec3<f32>, ray: FogRay) -> vec3<f32> {
@if(DISTANCE_FOG) {
    if view_bindings::fog.directional_light_color.a > 0.0 {
        return sample_sky(reflect(ray.direction, vec3<f32>(0.0, 1.0, 0.0)));
    }
    return view_bindings::fog.base_color.rgb;
} @else {
    return fallback;
}
}
