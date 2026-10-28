import bevy_pbr::render::{
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows,
};
// UV1 carries independent, normalized sky and block light. A small ambient floor
// keeps unlit caves readable; block light is independent of the sun and shadows.
fn sky_visibility(light: vec2<f32>) -> f32 {
    return 0.025 + 0.975 * pow(clamp(light.x, 0.0, 1.0), 2.2);
}

// Display-referred, added after exposure: torches look the same at noon and at
// midnight, while sun, moon and sky light follow the day cycle's exposure.
fn block_light(light: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(1.0, 0.80, 0.60)
        * (1.25 * pow(clamp(light.y, 0.0, 1.0), 2.2));
}

// Lambert irradiance (pre-exposure) from the sun and the moon. Only the light
// currently above the horizon casts shadows, so at most one map is sampled.
fn celestial_irradiance(
    normal: vec3<f32>,
    world_position: vec4<f32>,
    frag_coord: vec2<f32>,
    receive_shadows: bool,
) -> vec3<f32> {
    let view_z = dot(
        vec4<f32>(
            view_bindings::view.view_from_world[0].z,
            view_bindings::view.view_from_world[1].z,
            view_bindings::view.view_from_world[2].z,
            view_bindings::view.view_from_world[3].z,
        ),
        world_position,
    );
    var irradiance = vec3<f32>(0.0);
    for (var i = 0u; i < view_bindings::lights.n_directional_lights; i += 1u) {
        let light = &view_bindings::lights.directional_lights[i];
        let ndotl = dot(normal, (*light).direction_to_light);
        if ndotl <= 0.0 {
            continue;
        }
        var shadow = 1.0;
        if receive_shadows
                && ((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            shadow = shadows::fetch_directional_shadow(
                i,
                world_position,
                normal,
                view_z,
                frag_coord,
            );
        }
        // Occluded fragments contribute zero direct irradiance. In particular,
        // cave walls need no atmospheric transmittance lookup for this light.
        if shadow <= 0.0 {
            continue;
        }
        irradiance += (*light).color.rgb
            * (ndotl * shadow);
    }
    return irradiance * 0.318309886;
}

// The lean diffuse shading shared by terrain and opaque models, in display space.
fn voxel_diffuse(
    albedo: vec3<f32>,
    normal: vec3<f32>,
    world_position: vec4<f32>,
    frag_coord: vec2<f32>,
    voxel_light: vec2<f32>,
) -> vec3<f32> {
    let sky = sky_visibility(voxel_light) * view_bindings::view.exposure;
    let scene = view_bindings::lights.ambient_color.rgb
        + celestial_irradiance(normal, world_position, frag_coord, true);
    return albedo * (scene * sky + block_light(voxel_light));
}
