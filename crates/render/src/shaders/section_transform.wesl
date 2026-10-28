import bevy_pbr::render::mesh_functions;
@if(VOXEL_SECTIONS)
@group(2) @binding(32) var<storage, read> section_origins: array<vec4<f32>>;
fn world_from_local(instance: u32) -> mat4x4<f32> {
@if(VOXEL_SECTIONS) {
    return mat4x4<f32>(vec4<f32>(1.0,0.0,0.0,0.0), vec4<f32>(0.0,1.0,0.0,0.0), vec4<f32>(0.0,0.0,1.0,0.0), vec4<f32>(section_origins[instance].xyz,1.0));
} @else {
    return mesh_functions::get_world_from_local(instance);
}
}
fn world_normal(normal: vec3<f32>, instance: u32) -> vec3<f32> {
@if(VOXEL_SECTIONS) {
    return normalize(normal);
} @else {
    return mesh_functions::mesh_normal_local_to_world(normal,instance);
}
}
