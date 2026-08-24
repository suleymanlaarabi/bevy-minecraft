#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    mesh_view_types::{
        DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
        FOG_MODE_EXPONENTIAL,
        FOG_MODE_EXPONENTIAL_SQUARED,
        FOG_MODE_LINEAR,
    },
    shadows,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var blocks: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var blocks_sampler: sampler;

fn apply_voxel_fog(color: vec4<f32>, world_position: vec3<f32>) -> vec4<f32> {
#ifdef DISTANCE_FOG
    let fog = view_bindings::fog;
    let distance = length(world_position - view_bindings::view.world_position.xyz);
    var amount = 0.0;
    if fog.mode == FOG_MODE_LINEAR {
        amount = 1.0 - clamp((fog.be.y - distance) / (fog.be.y - fog.be.x), 0.0, 1.0);
    } else if fog.mode == FOG_MODE_EXPONENTIAL {
        amount = 1.0 - 1.0 / exp(distance * fog.be.x);
    } else if fog.mode == FOG_MODE_EXPONENTIAL_SQUARED {
        let density = distance * fog.be.x;
        amount = 1.0 - 1.0 / exp(density * density);
    }
    return vec4<f32>(mix(color.rgb, fog.base_color.rgb, amount * fog.base_color.a), color.a);
#else
    return color;
#endif
}

@fragment
fn fragment(
    in: VertexOutput,
) -> FragmentOutput {
#ifdef VERTEX_COLORS
    let layer = u32(in.color.a + 0.5);
    let tint = in.color.rgb;
#else
    let layer = 0u;
    let tint = vec3<f32>(1.0);
#endif

    var texture_color = textureSample(blocks, blocks_sampler, in.uv, layer);
    if layer == 8u {
        texture_color = vec4<f32>(
            mix(vec3<f32>(0.32), texture_color.rgb, texture_color.a),
            1.0,
        );
    }
    let albedo = texture_color.rgb * tint;
    let normal = normalize(in.world_normal);
    var irradiance = view_bindings::lights.ambient_color.rgb;

    if view_bindings::lights.n_directional_lights > 0u {
        let light = &view_bindings::lights.directional_lights[0u];
        let ndotl = max(dot(normal, (*light).direction_to_light), 0.0);
        var shadow = 1.0;
        if (((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            let view_z = dot(
                vec4<f32>(
                    view_bindings::view.view_from_world[0].z,
                    view_bindings::view.view_from_world[1].z,
                    view_bindings::view.view_from_world[2].z,
                    view_bindings::view.view_from_world[3].z,
                ),
                in.world_position,
            );
            shadow = shadows::fetch_directional_shadow(
                0u,
                in.world_position,
                normal,
                view_z,
                in.position.xy,
            );
        }
        irradiance += (*light).color.rgb * (ndotl * shadow * 0.318309886);
    }

    var out: FragmentOutput;
    out.color = apply_voxel_fog(
        vec4<f32>(albedo * irradiance * view_bindings::view.exposure, 1.0),
        in.world_position.xyz,
    );
    return out;
}
