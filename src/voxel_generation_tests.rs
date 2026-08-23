use super::*;

#[test]
fn generation_is_deterministic_diverse_and_seamless() {
    let settings = VoxelSettings::default();
    let generator = WorldGenerator::new(settings.seed);
    let origin = generate_chunk(IVec2::ZERO, &settings, &generator);
    let repeated = generate_chunk(IVec2::ZERO, &settings, &generator);
    let changed = generate_chunk(
        IVec2::ZERO,
        &settings,
        &WorldGenerator::new(settings.seed.wrapping_add(1)),
    );
    let mut seed_changed = false;
    for y in 0..settings.max_height {
        for z in 0..settings.chunk_size {
            for x in 0..settings.chunk_size {
                let position = IVec3::new(x, y, z);
                assert_eq!(origin.get(position), repeated.get(position));
                seed_changed |= origin.get(position) != changed.get(position);
            }
        }
    }
    assert!(seed_changed);

    let neighbor = generate_chunk(IVec2::X, &settings, &generator);
    for y in 0..settings.max_height {
        for z in 0..settings.chunk_size {
            assert_eq!(
                origin.sample(IVec3::new(settings.chunk_size, y, z)),
                neighbor.get(IVec3::new(0, y, z)).unwrap()
            );
        }
    }

    let mut biomes = [false; 4];
    let mut lowest = i32::MAX;
    let mut highest = i32::MIN;
    let mut snow_found = false;
    for z in (-2048..=2048).step_by(32) {
        for x in (-2048..=2048).step_by(32) {
            let column = generator.column(IVec2::new(x, z), &settings);
            biomes[column.biome as usize] = true;
            lowest = lowest.min(column.height);
            highest = highest.max(column.height);
            snow_found |= column.snow;
            assert!(column.height <= settings.max_height - TERRAIN_HEADROOM);
            if column.snow {
                assert!(column.height > sea_level(&settings) + 10);
            }
        }
    }
    assert!(biomes.into_iter().all(core::convert::identity));
    assert!(snow_found);
    assert!(lowest < sea_level(&settings));
    assert!(highest > sea_level(&settings) + 15);
}

#[test]
fn terrain_has_no_wall_like_height_jumps() {
    let settings = VoxelSettings::default();
    let generator = WorldGenerator::new(settings.seed);

    for z in -256..=256 {
        for x in -256..=256 {
            let position = IVec2::new(x, z);
            let height = generator.column(position, &settings).height;
            for neighbor in [position + IVec2::X, position + IVec2::Y] {
                let neighbor_height = generator.column(neighbor, &settings).height;
                assert!(
                    (height - neighbor_height).abs() <= 4,
                    "wall-like height jump from {position:?} ({height}) to {neighbor:?} ({neighbor_height})"
                );
            }
        }
    }
}

#[test]
fn tree_features_cross_chunk_edges_without_seams() {
    let settings = VoxelSettings::default();
    let generator = WorldGenerator::new(settings.seed);
    for z in -16..=16 {
        for x in -16..=16 {
            let chunk = IVec2::new(x, z);
            let left = generate_chunk(chunk, &settings, &generator);
            let right = generate_chunk(chunk + IVec2::X, &settings, &generator);
            let mut found_tree = false;
            for y in 0..settings.max_height {
                for local_z in 0..settings.chunk_size {
                    let outside = left.sample(IVec3::new(settings.chunk_size, y, local_z));
                    let inside = right.get(IVec3::new(0, y, local_z)).unwrap();
                    assert_eq!(outside, inside);
                    found_tree |= matches!(inside, VoxelKind::Wood | VoxelKind::Leaves);
                }
            }
            if found_tree {
                return;
            }
        }
    }
    panic!("default seed produced no tree crossing a tested chunk edge");
}
