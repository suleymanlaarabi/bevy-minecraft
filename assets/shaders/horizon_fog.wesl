import super::voxel_fog::apply_voxel_fog;
import super::fog_lut::horizon_haze;
import bevy_pbr::render::{forward_io::VertexOutput, mesh_view_bindings as view_bindings};

// Transparent, after Bevy's atmosphere: fade the lower background into the real
// horizon without writing a depth edge across the sky or hiding celestial disks.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
@if(DISTANCE_FOG) {
    if view_bindings::fog.directional_light_color.a > 0.0 {
        let direction = normalize(in.world_position.xyz - view_bindings::view.world_position.xyz);
        return horizon_haze(direction);
    }
}
    // The liquid dome is fully opaque and keeps the original immersion color.
    return apply_voxel_fog(vec4<f32>(0.0, 0.0, 0.0, 1.0), in.world_position.xyz);
}
