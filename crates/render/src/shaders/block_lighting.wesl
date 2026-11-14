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

// Shadow cascades are light-aligned boxes around the camera (`shadows.rs`), some
// cached from earlier frames. A fragment takes the first, sharpest box holding it,
// whatever its view depth, and fades into the next one near the box's faces; past
// the last box the shadow fades out.
const CLIPMAP_EDGE: f32 = 0.96;
const CLIPMAP_BLEND: f32 = 0.8;

fn clipmap_shadow(
    light_id: u32,
    world_position: vec4<f32>,
    normal: vec3<f32>,
    frag_coord: vec2<f32>,
) -> f32 {
    let count = view_bindings::lights.directional_lights[light_id].num_cascades;
    for (var i = 0u; i < count; i += 1u) {
        let clip = view_bindings::lights.directional_lights[light_id].cascades[i].clip_from_world
            * world_position;
        // 0 at the box's center, 1 on its faces (orthographic: w is 1, depth spans 0..1).
        let edge = max(max(abs(clip.x), abs(clip.y)), abs(clip.z * 2.0 - 1.0));
        if edge < CLIPMAP_EDGE {
            var shadow = shadows::sample_directional_cascade(
                light_id,
                i,
                world_position,
                normal,
                frag_coord,
            );
            if edge > CLIPMAP_BLEND {
                var next = 1.0;
                if i + 1u < count {
                    next = shadows::sample_directional_cascade(
                        light_id,
                        i + 1u,
                        world_position,
                        normal,
                        frag_coord,
                    );
                }
                shadow = mix(shadow, next, (edge - CLIPMAP_BLEND) / (CLIPMAP_EDGE - CLIPMAP_BLEND));
            }
            return shadow;
        }
    }
    return 1.0;
}

// Lambert irradiance (pre-exposure) from the sun and the moon. Only the light
// currently above the horizon casts shadows, so at most one map is sampled.
fn celestial_irradiance(
    normal: vec3<f32>,
    world_position: vec4<f32>,
    frag_coord: vec2<f32>,
    receive_shadows: bool,
) -> vec3<f32> {
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
            shadow = clipmap_shadow(i, world_position, normal, frag_coord);
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
    var scene = view_bindings::lights.ambient_color.rgb;
    // Without any sky light (caves, interiors) the sun and moon would be scaled to
    // the ambient floor, 2.5% of a light that is almost always shadowed there: the
    // filtered shadow lookup is skipped along with it.
    if voxel_light.x > 0.0 {
        scene += celestial_irradiance(normal, world_position, frag_coord, true);
    }
    return albedo * (scene * sky + block_light(voxel_light));
}
