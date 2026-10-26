use crate::generation::proto_chunk::GenerationCache;
use crate::lighting::engine::{BlockLightProvider, LightProvider};
use pumpkin_data::block_properties::{
    BlockProperties, GrassBlockLikeProperties, SnowLikeProperties,
};
use pumpkin_data::tag;
use pumpkin_data::{Block, BlockId, BlockState, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::random::RandomGenerator;

pub struct FreezeTopLayerFeature;

impl FreezeTopLayerFeature {
    pub fn generate<T: GenerationCache>(
        chunk: &mut T,
        _min_y: i8,
        _height: u16,
        _feature_name: pumpkin_data::placed_feature::PlacedFeature,
        _random: &mut RandomGenerator,
        pos: BlockPos,
    ) -> bool {
        let origin_x = pos.0.x;
        let origin_z = pos.0.z;

        for dx in 0..16i32 {
            for dz in 0..16i32 {
                let x = origin_x + dx;
                let z = origin_z + dz;

                let y = chunk.top_motion_blocking_block_height_exclusive(x, z);
                let below_y = y - 1;

                let top_vec = BlockPos::new(x, y, z).0;
                let below_vec = BlockPos::new(x, below_y, z).0;
                let below = GenerationCache::get_block_state(chunk, &below_vec);
                let below_block = below.to_block_id();

                let biome = chunk.get_biome_for_terrain_gen(x, y, z);

                // Freeze check
                if biome.weather.base_temperature() <= 0.15 && below_block == BlockId::WATER {
                    chunk.set_block_state(&below_vec, Block::ICE.default_state);
                    continue;
                }

                // Snow check
                let top_temp =
                    biome
                        .weather
                        .compute_temperature(x as f64, y, z as f64, chunk.get_sea_level());

                if biome.weather.has_precipitation() && top_temp < 0.15 {
                    let top_raw = GenerationCache::get_block_state(chunk, &top_vec);
                    // topPos must be air; belowPos must not be air (something to stand on)
                    // with a full top face (unless overridden), in darkness (vanilla
                    // only forms snow where block light is below 10).
                    if top_raw.to_state().is_air()
                        && !below.to_state().is_air()
                        && !below_block.has_tag(tag::Block::MINECRAFT_CANNOT_SUPPORT_SNOW_LAYER)
                        && (below_block.has_tag(tag::Block::MINECRAFT_SUPPORT_OVERRIDE_SNOW_LAYER)
                            || is_top_face_full(below.to_state())
                            || is_full_snow_layer(below))
                        && block_light_at(chunk, x, y, z) < 10
                    {
                        chunk.set_block_state(&top_vec, Block::SNOW.default_state);

                        // Update the `snowy` block-state property on the block below if it has one
                        if GrassBlockLikeProperties::handles_block_id(below_block) {
                            let block = below_block.to_block();
                            let mut props = GrassBlockLikeProperties::from_state_id(below);
                            props.snowy = true;
                            chunk.set_block_state(&below_vec, props.to_state_id(block).to_state());
                        }
                    }
                }
            }
        }

        true
    }
}

// `Block.isFaceFull` equivalent: the union of the collision shapes
// must fully cover the top face (per-shape checks miss multipart blocks
// whose boxes only cover it together).
fn is_top_face_full(state: &BlockState) -> bool {
    let mut boxes = Vec::new();
    let mut xs = vec![0.0, 1.0];
    let mut zs = vec![0.0, 1.0];
    for shape in state.get_block_collision_shapes() {
        if shape.max.y < 1.0 {
            continue;
        }
        let (x0, x1) = (shape.min.x.max(0.0), shape.max.x.min(1.0));
        let (z0, z1) = (shape.min.z.max(0.0), shape.max.z.min(1.0));
        if x0 < x1 && z0 < z1 {
            xs.push(x0);
            xs.push(x1);
            zs.push(z0);
            zs.push(z1);
            boxes.push((x0, x1, z0, z1));
        }
    }
    if boxes.is_empty() {
        return false;
    }
    xs.sort_by(f64::total_cmp);
    zs.sort_by(f64::total_cmp);
    xs.dedup();
    zs.dedup();
    xs.windows(2).all(|xw| {
        zs.windows(2).all(|zw| {
            boxes
                .iter()
                .any(|&(x0, x1, z0, z1)| x0 <= xw[0] && xw[1] <= x1 && z0 <= zw[0] && zw[1] <= z1)
        })
    })
}

// Vanilla's `SnowLayerBlock.canSurvive` also accepts a full eight-layer snow
// block as support, even though its collision shape is only 7/8 blocks tall.
fn is_full_snow_layer(state: BlockStateId) -> bool {
    state.to_block_id() == BlockId::SNOW && SnowLikeProperties::from_state_id(state).layers == 8
}

/// Block light at a world position during generation. Sections without
/// computed light read as 0, matching vanilla worldgen behavior.
fn block_light_at<T: GenerationCache>(chunk: &T, x: i32, y: i32, z: i32) -> u8 {
    let Some(proto) = chunk.get_chunk(x >> 4, z >> 4) else {
        return 0;
    };
    let Ok(section_idx) = usize::try_from((y - i32::from(proto.bottom_y())) / 16) else {
        return 0;
    };
    BlockLightProvider::get_light_proto(
        proto,
        section_idx,
        (x & 15) as usize,
        (y & 15) as usize,
        (z & 15) as usize,
    )
}

