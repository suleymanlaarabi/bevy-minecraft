import super::block_lighting::{sky_visibility, block_irradiance};
import super::voxel_fog::apply_voxel_fog;
import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_view_bindings as view_bindings,
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT,
    shadows,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0)
var blocks: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1)
var blocks_sampler: sampler;

@fragment
fn fragment(
    in: VertexOutput,
) -> FragmentOutput {
@if(VERTEX_COLORS)
    let layer = u32(in.color.a + 0.5);
@if(VERTEX_COLORS)
    let tint = in.color.rgb;
@if(!VERTEX_COLORS)
    let layer = 0u;
@if(!VERTEX_COLORS)
    let tint = vec3<f32>(1.0);

    var texture_color = textureSample(blocks, blocks_sampler, in.uv, layer);
    // Terrain is opaque: close leaf cutouts (any layer with alpha) with a dark tone.
    if texture_color.a < 1.0 {
        texture_color = vec4<f32>(
            mix(vec3<f32>(0.32), texture_color.rgb, texture_color.a),
            1.0,
        );
    }
    let albedo = texture_color.rgb * tint;
    let normal = normalize(in.world_normal);
@if(VERTEX_UVS_B)
    let voxel_light = in.uv_b;
@if(!VERTEX_UVS_B)
    let voxel_light = vec2<f32>(1.0, 0.0);
    let sky = sky_visibility(voxel_light);
    var irradiance = view_bindings::lights.ambient_color.rgb * sky
        + block_irradiance(voxel_light);

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
        irradiance += (*light).color.rgb * (ndotl * shadow * sky * 0.318309886);
    }

    var out: FragmentOutput;
    out.color = apply_voxel_fog(
        vec4<f32>(albedo * irradiance * view_bindings::view.exposure, 1.0),
        in.world_position.xyz,
    );
    return out;
}
