import bevy_pbr::render::mesh_functions;
// Section draws are gathered into one index list per view (`section_compact.wgsl`),
// so the instance no longer names the section: each 64-vertex page of a pass's
// vertex arena holds the origin of the section that owns it.
@if(VOXEL_SECTIONS)
@group(2) @binding(32) var<storage, read> section_pages: array<vec4<f32>>;
fn world_from_local(instance: u32, vertex: u32) -> mat4x4<f32> {
@if(VOXEL_SECTIONS) {
    return mat4x4<f32>(vec4<f32>(1.0,0.0,0.0,0.0), vec4<f32>(0.0,1.0,0.0,0.0), vec4<f32>(0.0,0.0,1.0,0.0), vec4<f32>(section_pages[vertex >> 6u].xyz,1.0));
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
