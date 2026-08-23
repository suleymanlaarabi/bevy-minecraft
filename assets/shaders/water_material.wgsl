#import bevy_pbr::{
    mesh_view_bindings::globals,
    pbr_fragment::pbr_input_from_standard_material,
}

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

struct WaterSettings {
    directions: vec4<f32>,
    frequency_speed: vec4<f32>,
    strength: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> water: WaterSettings;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let position = pbr_input.world_position.xz;
    let first_direction = normalize(water.directions.xy);
    let second_direction = normalize(water.directions.zw);
    let first_phase = dot(position, first_direction) * water.frequency_speed.x
        + globals.time * water.frequency_speed.z;
    let second_phase = dot(position, second_direction) * water.frequency_speed.y
        + globals.time * water.frequency_speed.w;
    let slope = first_direction
            * cos(first_phase)
            * water.strength.x
            * water.frequency_speed.x
        + second_direction
            * cos(second_phase)
            * water.strength.y
            * water.frequency_speed.y;

    var wave_normal = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    if !is_front {
        wave_normal = -wave_normal;
    }
    pbr_input.N = wave_normal;

    let facing = clamp(dot(pbr_input.N, pbr_input.V), 0.0, 1.0);
    let fresnel = pow(1.0 - facing, 5.0);
    let ripple = 0.5 + 0.5 * sin(first_phase + second_phase);
    let surface_color = pbr_input.material.base_color;
    let deep_color = surface_color.rgb * vec3<f32>(0.42, 0.35, 0.47);
    pbr_input.material.base_color = vec4<f32>(
        mix(surface_color.rgb, deep_color, 0.28 + ripple * 0.10),
        mix(surface_color.a * 0.86, min(surface_color.a + 0.16, 1.0), fresnel),
    );
    pbr_input.material.perceptual_roughness = mix(
        pbr_input.material.perceptual_roughness,
        0.035,
        fresnel,
    );

#ifdef PREPASS_PIPELINE
    return deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
#endif
}
