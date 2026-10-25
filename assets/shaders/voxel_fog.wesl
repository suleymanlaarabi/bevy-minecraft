import super::fog_lut::{has_fog_lut, natural_fog_color, fog_before_atmosphere, sample_fog_sky};
import super::block_lighting::atmosphere_transmittance;
import bevy_pbr::render::{
    mesh_view_bindings as view_bindings,
    mesh_view_types::{FOG_MODE_EXPONENTIAL, FOG_MODE_EXPONENTIAL_SQUARED, FOG_MODE_LINEAR},
};

// Shared by terrain, models, water and the horizon skirt so all blend into the same
// distance fog. Clouds reach past it: they fade out on their own.
// Open-air fog samples the physical sky; liquids keep their own spherical falloff.
fn apply_voxel_fog(color: vec4<f32>, world_position: vec3<f32>) -> vec4<f32> {
@if(DISTANCE_FOG) {
    let fog = view_bindings::fog;
    let offset = world_position - view_bindings::view.world_position.xyz;
    let distance = length(offset);
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
    if amount <= 0.0 { return color; }
    var fog_color = fog.base_color.rgb;
    let direction = offset / max(distance, 1e-4);
    let physical_sky = open_air && has_fog_lut();
    if physical_sky {
        fog_color = natural_fog_color(direction);
@if(FOG_BEFORE_ATMOSPHERE) {
        fog_color = fog_before_atmosphere(fog_color, world_position);
}
    }
    // Forward scattering around the sun and moon, reddened by the atmosphere at dusk.
    if open_air && !physical_sky {
        let camera = view_bindings::view.world_position.xyz;
        var scattering = vec3<f32>(0.0);
        for (var i = 0u; i < view_bindings::lights.n_directional_lights; i += 1u) {
            let light = &view_bindings::lights.directional_lights[i];
            let glow = pow(
                max(dot(direction, (*light).direction_to_light), 0.0),
                fog.directional_light_exponent,
            );
            if glow > 0.0 {
                scattering += (*light).color.rgb
                    * atmosphere_transmittance(
                        camera,
                        (*light).direction_to_light,
                        (*light).sun_disk_angular_size,
                    )
                    * glow;
            }
        }
        fog_color += scattering
            * view_bindings::view.exposure
            * fog.directional_light_color.rgb
            * fog.directional_light_color.a;
    }
    return vec4<f32>(mix(color.rgb, fog_color, amount * fog.base_color.a), color.a);
} @else {
    return color;
}
}

// Sky color reflected by water at grazing angles, in output space.
fn sky_reflection(fallback: vec3<f32>, world_position: vec3<f32>) -> vec3<f32> {
@if(DISTANCE_FOG) {
    if view_bindings::fog.directional_light_color.a > 0.0 && has_fog_lut() {
        let incoming = normalize(world_position - view_bindings::view.world_position.xyz);
        return sample_fog_sky(reflect(incoming, vec3<f32>(0.0, 1.0, 0.0)));
    }
    return view_bindings::fog.base_color.rgb;
} @else {
    return fallback;
}
}
