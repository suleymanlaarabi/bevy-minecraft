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
