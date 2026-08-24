#import bevy_pbr::{
    ambient,
    lighting,
    lighting::LAYER_BASE,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    pbr_functions::{calculate_F0, calculate_F0_dielectric},
    pbr_types::PbrInput,
    shadows,
}

// The water still uses the StandardMaterial input. Keep its specialized
// ambient-and-sun lighting path separate from the lean opaque voxel material.
fn apply_voxel_lighting(in: PbrInput) -> vec4<f32> {
    let base_color = in.material.base_color;
    let metallic = in.material.metallic;
    let perceptual_roughness = in.material.perceptual_roughness;
    let roughness = lighting::perceptualRoughnessToRoughness(perceptual_roughness);
    let reflectance = in.material.reflectance;
    let diffuse_color = base_color.rgb * (1.0 - metallic);
    let F0 = calculate_F0(base_color.rgb, metallic, reflectance);
    let NdotV = max(dot(in.N, in.V), 0.0001);

    var lighting_input: lighting::LightingInput;
    lighting_input.layers[LAYER_BASE].NdotV = NdotV;
    lighting_input.layers[LAYER_BASE].N = in.N;
    lighting_input.layers[LAYER_BASE].R = reflect(-in.V, in.N);
    lighting_input.layers[LAYER_BASE].perceptual_roughness = perceptual_roughness;
    lighting_input.layers[LAYER_BASE].roughness = roughness;
    lighting_input.P = in.world_position.xyz;
    lighting_input.V = in.V;
    lighting_input.diffuse_color = diffuse_color;
    lighting_input.metallic = metallic;
    lighting_input.F0_dielectric = calculate_F0_dielectric(reflectance);
    lighting_input.F0_metallic = base_color.rgb;
    lighting_input.F_ab = lighting::F_AB(perceptual_roughness, NdotV);

    let view_z = dot(
        vec4<f32>(
            view_bindings::view.view_from_world[0].z,
            view_bindings::view.view_from_world[1].z,
            view_bindings::view.view_from_world[2].z,
            view_bindings::view.view_from_world[3].z,
        ),
        in.world_position,
    );

    var direct_light = vec3<f32>(0.0);
    let n_directional_lights = view_bindings::lights.n_directional_lights;
    for (var i = 0u; i < n_directional_lights; i += 1u) {
        let light = &view_bindings::lights.directional_lights[i];
        var shadow = 1.0;
        if ((in.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u
                && ((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            shadow = shadows::fetch_directional_shadow(
                i,
                in.world_position,
                in.world_normal,
                view_z,
                in.frag_coord.xy,
            );
        }
        direct_light += lighting::directional_light(i, &lighting_input, true) * shadow;
    }

    let indirect_light = ambient::ambient_light(
        in.world_position,
        in.N,
        in.V,
        NdotV,
        diffuse_color,
        F0,
        perceptual_roughness,
        in.diffuse_occlusion,
    );
    let emissive = in.material.emissive;
    let emissive_light = emissive.rgb
        * base_color.a
        * mix(1.0, view_bindings::view.exposure, emissive.a);

    return vec4<f32>(
        view_bindings::view.exposure * (direct_light + indirect_light) + emissive_light,
        base_color.a,
    );
}
