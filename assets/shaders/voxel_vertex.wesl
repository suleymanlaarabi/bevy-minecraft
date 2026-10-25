// Decodes the packed terrain vertex written by `TerrainGeometry::push_quad`.
struct VoxelVertex {
    // Section-local position.
    position: vec3<f32>,
    // Face direction: axis * 2 + positive.
    direction: u32,
    // Greedy cube appearance (`models::Appearance`).
    appearance: u32,
    // Normalized sky and block light.
    light: vec2<f32>,
}

struct PackedVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) voxel: vec2<u32>,
}

fn decode(packed: vec2<u32>) -> VoxelVertex {
    var vertex: VoxelVertex;
    vertex.position = vec3<f32>(
        vec3<u32>(packed.x, packed.x >> 5u, packed.x >> 10u) & vec3<u32>(31u),
    );
    vertex.direction = (packed.x >> 15u) & 7u;
    vertex.appearance = (packed.x >> 18u) & 16383u;
    vertex.light = vec2<f32>(vec2<u32>(packed.y, packed.y >> 6u) & vec2<u32>(63u)) / 60.0;
    return vertex;
}

fn face_normal(direction: u32) -> vec3<f32> {
    var normal = vec3<f32>(0.0);
    normal[direction >> 1u] = select(-1.0, 1.0, (direction & 1u) != 0u);
    return normal;
}

// The texture coordinates the mesher used to store per quad, up to whole blocks,
// which the repeating sampler ignores.
fn block_uv(position: vec3<f32>, axis: u32) -> vec2<f32> {
    if axis == 0u {
        return vec2<f32>(position.z, -position.y);
    }
    if axis == 1u {
        return position.zx;
    }
    return vec2<f32>(position.x, -position.y);
}

// Index into the material's face table: six faces per appearance, by direction.
fn face_index(vertex: VoxelVertex) -> u32 {
    return vertex.appearance * 6u + vertex.direction;
}

// Dihedral map from block UVs to the model's face UVs (`models::transform_uv`):
// bit 0 swaps u and v, bits 1 and 2 negate them. Integer maps keep tiling seamless.
fn transform_uv(uv: vec2<f32>, transform: u32) -> vec2<f32> {
    let swapped = select(uv, uv.yx, (transform & 1u) != 0u);
    return vec2<f32>(
        select(swapped.x, -swapped.x, (transform & 2u) != 0u),
        select(swapped.y, -swapped.y, (transform & 4u) != 0u),
    );
}

// Face table entries pack the texture layer in the low 12 bits and the UV
// transform above them.
fn face_layer(packed: u32) -> u32 {
    return packed & 4095u;
}
fn face_transform(packed: u32) -> u32 {
    return packed >> 12u;
}

