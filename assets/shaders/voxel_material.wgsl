#import bevy_pbr::pbr_fragment::pbr_input_from_standard_material
#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var blocks: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101)
var blocks_sampler: sampler;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

#ifdef VERTEX_COLORS
    let layer = u32(in.color.a + 0.5);
    let tint = vec4<f32>(in.color.rgb, 1.0);
#else
    let layer = 0u;
    let tint = vec4<f32>(1.0);
#endif

    var texture_color = textureSample(blocks, blocks_sampler, in.uv, layer);
    if layer == 8u {
        texture_color = vec4<f32>(
            mix(vec3<f32>(0.32), texture_color.rgb, texture_color.a),
            1.0,
        );
    }
    pbr_input.material.base_color = texture_color * tint;

#ifdef PREPASS_PIPELINE
    return deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
#endif
}
