use super::coordinates::*;
use super::storage::Layer;

use super::*;

#[test]
fn lighting_updates_only_dirty_changed_sections() {
    let mut chunk = ChunkVoxels::filled(VoxelKind::Air);
    chunk.dirty_sections = 0;
    let mut light = chunk.light.clone();
    light.set(cell_index(IVec3::new(3, 40, 5)).unwrap(), 0x5c);
    assert_eq!(chunk.replace_light(light.clone()), 0b1110);
    assert_eq!(chunk.dirty_sections, 0b1110);
    assert_eq!(chunk.content_revision, 0);
    assert!(!chunk.modified);
    assert_eq!(chunk.revision, 1);
    assert_eq!(chunk.replace_light(light), 0);
    assert_eq!(chunk.revision, 1);
}

#[test]
fn edits_advance_revision_and_only_dirty_required_outputs() {
    let mut chunk = ChunkVoxels::filled(VoxelKind::Air);
    chunk.dirty_sections = 0;
    chunk.collider_dirty = false;

    assert!(chunk.set(IVec3::new(0, 15, 0), VoxelKind::Stone));
    assert_eq!(chunk.revision, 1);
    assert_eq!(chunk.dirty_sections, 0b11);
    assert!(chunk.collider_dirty);
    assert!(chunk.modified);

    chunk.dirty_sections = 0;
    chunk.collider_dirty = false;
    assert!(!chunk.set(IVec3::new(0, 15, 0), VoxelKind::Stone));
    assert_eq!(chunk.revision, 1);
    assert_eq!(chunk.dirty_sections, 0);

    assert!(chunk.set(IVec3::new(1, 4, 1), VoxelKind::Water));
    assert_eq!(chunk.dirty_sections, 0b01);
    assert!(!chunk.collider_dirty);
}

#[test]
fn uniform_sections_store_no_cells_until_edited() {
    let mut cells = vec![VoxelKind::Air; CHUNK_VOLUME];
    cells[..SECTION_VOLUME].fill(VoxelKind::Stone);
    cells[SECTION_VOLUME + 7] = VoxelKind::Dirt;
    let layer = Layer::from_cells(&cells);
    assert_eq!(layer.section(0).uniform(), Some(VoxelKind::Stone));
    assert_eq!(layer.section(1).uniform(), None);
    assert!(
        (2..SECTION_COUNT).all(|section| layer.section(section).uniform() == Some(VoxelKind::Air))
    );
    assert_eq!(&*layer.to_cells(), &cells[..]);

    // Copy-on-write: the edit leaves a snapshot and the other sections untouched.
    let mut edited = layer.clone();
    assert!(edited.set(5, VoxelKind::Dirt));
    assert!(!edited.set(5, VoxelKind::Dirt));
    assert_eq!(layer.get(5), VoxelKind::Stone);
    assert_eq!(edited.get(5), VoxelKind::Dirt);
    assert_eq!(edited.changed_sections(&layer), 0b1);
}

#[test]
fn cell_indices_match_the_pumpkin_layout() {
    let local = IVec3::new(3, 200, 11);
    let index = cell_index(local).unwrap();
    assert_eq!(index, 3 + 16 * (11 + 16 * 200));
    assert_eq!(cell_position(index), local);
    for outside in [
        IVec3::new(-1, 0, 0),
        IVec3::new(16, 0, 0),
        IVec3::new(0, 384, 0),
    ] {
        assert_eq!(cell_index(outside), None);
    }
    assert_eq!(
        split_world_position(IVec3::new(-1, 70, 33)),
        (IVec2::new(-1, 2), IVec3::new(15, 70, 1))
    );
}

#[test]
fn colliders_keep_only_the_exposed_shell() {
    let mut chunk = ChunkVoxels::filled(VoxelKind::Air);
    for y in 0..4 {
        for z in 0..16 {
            for x in 0..16 {
                chunk.set(IVec3::new(x, y, z), VoxelKind::Stone);
            }
        }
    }
    let voxels = chunk.collider_voxels();
    // The top layer, the bottom layer and the chunk border of the two between.
    assert_eq!(voxels.len(), 2 * 256 + 2 * 60);
    assert!(!voxels.contains(&IVec3::new(5, 1, 5)));
    assert!(voxels.contains(&IVec3::new(0, 1, 5)));
    assert!(voxels.contains(&IVec3::new(5, 3, 5)));
}
