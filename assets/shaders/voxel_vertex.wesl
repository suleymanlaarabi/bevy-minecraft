// Decodes the packed terrain vertex written by `TerrainGeometry::push_quad`.
struct VoxelVertex {
    // Section-local position.
    position: vec3<f32>,
    // Face direction: axis * 2 + positive.
    direction: u32,
    kind: u32,
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
    vertex.kind = (packed.x >> 18u) & 255u;
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

// Index into the material's face table: top, side and bottom slots per block.
fn face_index(vertex: VoxelVertex) -> u32 {
    let axis = vertex.direction >> 1u;
    let top = (vertex.direction & 1u) != 0u;
    return vertex.kind * 3u + select(1u, select(2u, 0u, top), axis == 1u);
}
