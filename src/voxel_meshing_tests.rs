use super::*;

#[test]
fn water_is_greedy_and_non_colliding() {
    let mut cells = vec![VoxelKind::Air; 8];
    cells[..4].fill(VoxelKind::Water);
    cells[4] = VoxelKind::Leaves;
    cells[5] = VoxelKind::Wood;
    let voxels = ChunkVoxels::generated(2, 2, cells, vec![VoxelKind::Air; 16]);
    let meshes = build_chunk_mesh(&voxels);
    assert_eq!(meshes.1.unwrap().indices().unwrap().len(), 6);
    assert_eq!(voxels.solid_positions(), vec![IVec3::new(1, 1, 0)]);
}

#[test]
fn terrain_mesh_carries_repeating_texture_coordinates() {
    let mut cells = vec![VoxelKind::Air; 8];
    cells[0] = VoxelKind::Grass;
    let voxels = ChunkVoxels::generated(2, 2, cells, vec![VoxelKind::Air; 16]);
    let mesh = build_chunk_mesh(&voxels).0;
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
    let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
        mesh.attribute(Mesh::ATTRIBUTE_UV_0)
    else {
        panic!("terrain mesh has no Float32x2 UV attribute");
    };
    let bevy::mesh::VertexAttributeValues::Float32x3(positions) = positions else {
        panic!("terrain mesh has no Float32x3 position attribute");
    };
    assert_eq!(positions.len(), uvs.len());
    assert!(
        uvs.iter()
            .all(|uv| uv.iter().all(|value| value.is_finite()))
    );
}

#[test]
fn vertical_face_textures_keep_world_up() {
    assert_eq!(
        quad_uvs(0, [0, 2, 0], [0, 0, 3]),
        [[0.0, 2.0], [0.0, 0.0], [3.0, 0.0], [3.0, 2.0]]
    );
    assert_eq!(
        quad_uvs(2, [3, 0, 0], [0, 2, 0]),
        [[0.0, 2.0], [3.0, 2.0], [3.0, 0.0], [0.0, 0.0]]
    );
}
