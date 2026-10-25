import super::voxel_fog::apply_voxel_fog;
import bevy_pbr::forward_io::VertexOutput;

// The horizon skirt and the liquid dome sit past the end of the fog, so this is the
// fog's color alone, sun scattering included.
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return apply_voxel_fog(vec4<f32>(0.0, 0.0, 0.0, 1.0), in.world_position.xyz);
}
