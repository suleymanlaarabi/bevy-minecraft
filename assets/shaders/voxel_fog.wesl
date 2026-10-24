import bevy_pbr::render::{
    mesh_view_bindings as view_bindings,
    mesh_view_types::{FOG_MODE_EXPONENTIAL, FOG_MODE_EXPONENTIAL_SQUARED, FOG_MODE_LINEAR},
};

// Shared by terrain and water so both blend into the same distance fog.
fn apply_voxel_fog(color: vec4<f32>, world_position: vec3<f32>) -> vec4<f32> {
@if(DISTANCE_FOG) {
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
} @else {
    return color;
}
}
