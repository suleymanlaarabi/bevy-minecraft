//! Generation identities and shared immutable output.
use crate::world::{BlockAccessor, WorldPortalExt};
use pumpkin_data::{
    Block, BlockState, BlockStateId, Mirror, Rotation, dimension::Dimension as PumpkinDimension,
};
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
#[path = "embedding_lighting.rs"]
mod lighting;
pub use crate::chunk::io::ChunkStorage;
pub const PUMPKIN_COMMIT: &str = "8ca5cb443272e0ac4f59b4c2ac059bdc72c2a22b";
pub const MINECRAFT_VERSION: &str = crate::CURRENT_MC_VERSION;
pub const FAST_GENERATION_VERSION: u32 = 2;
pub const GENERATOR_VERSION: &str = "hollow-bevy-2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}
impl Dimension {
    pub(crate) fn pumpkin(self) -> PumpkinDimension {
        match self {
            Self::Overworld => PumpkinDimension::OVERWORLD,
            Self::Nether => PumpkinDimension::THE_NETHER,
            Self::End => PumpkinDimension::THE_END,
        }
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum TerrainQuality {
    Vanilla,
    #[default]
    Fast,
}

/// Immutable section snapshot. Edits clone only the affected sections.
#[derive(Debug, Clone)]
pub struct ChunkSnapshot {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub min_y: i32,
    pub height: i32,
    pub states: crate::sections::Layer<crate::VoxelState>,
    pub light: crate::sections::Layer<u8>,
}

impl ChunkSnapshot {
    pub(crate) fn from_proto(chunk: &crate::ProtoChunk, min_y: i32, height: i32) -> Self {
        let (chunk_x, chunk_z) = (chunk.x, chunk.z);
        use crate::{
            VoxelState,
            sections::{Layer, Section},
        };
        let states = Layer::from_sections((0..height as usize / 16).map(|section| {
            let cells: [_; 4096] = std::array::from_fn(|i| {
                let position = Vector3::new(
                    chunk_x * 16 + (i & 15) as i32,
                    min_y + (section * 16 + (i >> 8)) as i32,
                    chunk_z * 16 + ((i >> 4) & 15) as i32,
                );
                VoxelState::from_id(chunk.get_block_state(&position).as_u16()).unwrap()
            });
            Section::from_cells(&cells)
        }));
        Self {
            chunk_x,
            chunk_z,
            min_y,
            height,
            states,
            light: lighting::working_sections(&chunk.light, height as usize / 16),
        }
    }

    pub fn block_state_id(&self, x: u8, y: i32, z: u8) -> Option<u16> {
        if x >= 16 || z >= 16 || y < self.min_y || y >= self.min_y + self.height {
            return None;
        }
        Some(
            self.states
                .get((y - self.min_y) as usize * 256 + z as usize * 16 + x as usize)
                .id(),
        )
    }
}

/// Registry adapter matching the one Pumpkin uses in its vanilla parity tests.
/// Entity spawning is intentionally disabled because this crate returns block-only chunks.
pub(crate) struct VanillaBlockRegistry;

impl WorldPortalExt for VanillaBlockRegistry {
    /// Reject unsupported plants before worldgen writes them. Exceptions precede
    /// the flower/sapling tags because those tags also contain specialized plants.
    fn can_place_at(
        &self,
        block: &Block,
        state: &BlockState,
        world: &dyn BlockAccessor,
        pos: &BlockPos,
    ) -> bool {
        use pumpkin_data::{
            BlockDirection::{Down, Up},
            BlockId,
            block_properties::{
                COLLISION_SHAPES, DoubleBlockHalf, LadderLikeProperties,
                MangrovePropaguleLikeProperties, SmallDripleafLikeProperties,
                TallSeagrassLikeProperties,
            },
            tag::{self, Taggable},
        };

        let below = world.get_block_state(&pos.down());
        let below_id = below.id.to_block_id();
        let water = |s: &BlockState| {
            matches!(
                s.id.to_block_id(),
                BlockId::WATER
                    | BlockId::BUBBLE_COLUMN
                    | BlockId::SEAGRASS
                    | BlockId::TALL_SEAGRASS
                    | BlockId::KELP
                    | BlockId::KELP_PLANT
            ) || s.is_waterlogged()
        };
        let source_water = |s: &BlockState| {
            water(s)
                && (s.id.to_block_id() != BlockId::WATER || s.id == Block::WATER.default_state.id)
        };
        match block.id {
            BlockId::WITHER_ROSE => below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_WITHER_ROSE),
            BlockId::AZALEA | BlockId::FLOWERING_AZALEA => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_AZALEA)
            }
            BlockId::MANGROVE_PROPAGULE => {
                if MangrovePropaguleLikeProperties::from_state_id(state.id).hanging {
                    world
                        .get_block(&pos.up())
                        .has_tag(&tag::Block::MINECRAFT_SUPPORTS_HANGING_MANGROVE_PROPAGULE)
                } else {
                    below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_MANGROVE_PROPAGULE)
                }
            }
            BlockId::TALL_GRASS
            | BlockId::LARGE_FERN
            | BlockId::SUNFLOWER
            | BlockId::LILAC
            | BlockId::ROSE_BUSH
            | BlockId::PEONY
            | BlockId::PITCHER_PLANT
            | BlockId::TALL_SEAGRASS => {
                if TallSeagrassLikeProperties::from_state_id(state.id).half
                    == DoubleBlockHalf::Upper
                {
                    below_id == block.id
                        && TallSeagrassLikeProperties::from_state_id(below.id).half
                            == DoubleBlockHalf::Lower
                } else if block.id == BlockId::TALL_SEAGRASS {
                    below.is_side_solid(Up)
                        && !below_id.has_tag(tag::Block::MINECRAFT_CANNOT_SUPPORT_SEAGRASS)
                        && source_water(world.get_block_state(pos))
                } else {
                    below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_VEGETATION)
                }
            }
            BlockId::SMALL_DRIPLEAF => {
                if SmallDripleafLikeProperties::from_state_id(state.id).half
                    == DoubleBlockHalf::Upper
                {
                    below_id == block.id
                        && SmallDripleafLikeProperties::from_state_id(below.id).half
                            == DoubleBlockHalf::Lower
                } else {
                    below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_SMALL_DRIPLEAF)
                        || (source_water(world.get_block_state(pos))
                            && below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_VEGETATION))
                }
            }
            // Bushes and flower beds are not included in the small-flower tag.
            BlockId::SHORT_GRASS
            | BlockId::FERN
            | BlockId::SWEET_BERRY_BUSH
            | BlockId::BUSH
            | BlockId::RED_SHRUB
            | BlockId::FIREFLY_BUSH
            | BlockId::PINK_PETALS
            | BlockId::WILDFLOWERS => below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_VEGETATION),
            BlockId::DEAD_BUSH | BlockId::SHORT_DRY_GRASS | BlockId::TALL_DRY_GRASS => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_DRY_VEGETATION)
            }
            BlockId::LEAF_LITTER => below.is_side_solid(Up),
            BlockId::SEAGRASS => {
                below.is_side_solid(Up)
                    && !below_id.has_tag(tag::Block::MINECRAFT_CANNOT_SUPPORT_SEAGRASS)
            }
            BlockId::SEA_PICKLE => {
                below.is_side_solid(Up)
                    || below.collision_shapes.iter().any(|&shape| {
                        let b = &COLLISION_SHAPES[shape as usize];
                        b.min.y < 1.0 && b.max.y >= 1.0
                    })
            }
            BlockId::HANGING_ROOTS => world.get_block_state(&pos.up()).is_side_solid(Down),
            BlockId::SPORE_BLOSSOM => {
                let above = world.get_block_state(&pos.up());
                above.is_center_solid(Down)
                    && !above
                        .id
                        .to_block_id()
                        .has_tag(tag::Block::MINECRAFT_UNSTABLE_BOTTOM_CENTER)
                    && !water(world.get_block_state(pos))
            }
            BlockId::BIG_DRIPLEAF => {
                matches!(below_id, BlockId::BIG_DRIPLEAF | BlockId::BIG_DRIPLEAF_STEM)
                    || below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_BIG_DRIPLEAF)
            }
            BlockId::BIG_DRIPLEAF_STEM => {
                (below_id == BlockId::BIG_DRIPLEAF_STEM
                    || below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_BIG_DRIPLEAF))
                    && matches!(
                        world.get_block(&pos.up()).id,
                        BlockId::BIG_DRIPLEAF | BlockId::BIG_DRIPLEAF_STEM
                    )
            }
            BlockId::KELP | BlockId::KELP_PLANT => {
                !below_id.has_tag(tag::Block::MINECRAFT_CANNOT_SUPPORT_KELP)
                    && (matches!(below_id, BlockId::KELP | BlockId::KELP_PLANT)
                        || below.is_side_solid(Up))
            }
            BlockId::CACTUS => {
                (below_id == BlockId::CACTUS
                    || below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_CACTUS))
                    && !world.get_block_state(&pos.up()).is_liquid()
                    && pumpkin_data::BlockDirection::horizontal()
                        .into_iter()
                        .all(|dir| {
                            let neighbor = world.get_block_state(&pos.offset(dir.to_offset()));
                            !neighbor.is_solid() && neighbor.id.to_block_id() != BlockId::LAVA
                        })
            }
            BlockId::CACTUS_FLOWER => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORT_OVERRIDE_CACTUS_FLOWER)
                    || below.is_center_solid(Up)
            }
            BlockId::SUGAR_CANE => {
                below_id == BlockId::SUGAR_CANE
                    || (below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_SUGAR_CANE)
                        && pumpkin_data::BlockDirection::horizontal()
                            .into_iter()
                            .any(|dir| {
                                let neighbor =
                                    world.get_block_state(&pos.down().offset(dir.to_offset()));
                                water(neighbor)
                                    || neighbor.id.to_block_id().has_tag(
                                        tag::Block::MINECRAFT_SUPPORTS_SUGAR_CANE_ADJACENTLY,
                                    )
                            }))
            }
            BlockId::BAMBOO | BlockId::BAMBOO_SAPLING => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_BAMBOO)
            }
            BlockId::WARPED_FUNGUS => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_WARPED_FUNGUS)
            }
            BlockId::CRIMSON_FUNGUS => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_CRIMSON_FUNGUS)
            }
            BlockId::WARPED_ROOTS => below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_WARPED_ROOTS),
            BlockId::CRIMSON_ROOTS => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_CRIMSON_ROOTS)
            }
            BlockId::NETHER_SPROUTS => {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_NETHER_SPROUTS)
            }
            BlockId::NETHER_WART => below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_NETHER_WART),
            BlockId::BROWN_MUSHROOM | BlockId::RED_MUSHROOM => {
                // Lighting is unavailable until the later lighting stage.
                below_id.has_tag(tag::Block::MINECRAFT_OVERRIDES_MUSHROOM_LIGHT_REQUIREMENT)
                    || below.is_solid_render()
            }
            BlockId::LILY_PAD => {
                let current = world.get_block_state(pos);
                (water(below) || below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_LILY_PAD))
                    && !current.is_liquid()
                    && !water(current)
            }
            _ if block.has_tag(&tag::Block::MINECRAFT_SMALL_FLOWERS)
                || block.has_tag(&tag::Block::MINECRAFT_SAPLINGS) =>
            {
                below_id.has_tag(tag::Block::MINECRAFT_SUPPORTS_VEGETATION)
            }
            _ if block.has_tag(&tag::Block::MINECRAFT_CORALS) => below.is_side_solid(Up),
            _ if block.has_tag(&tag::Block::MINECRAFT_WALL_CORALS) => {
                let facing = pumpkin_data::BlockDirection::from_cardinal_direction(
                    LadderLikeProperties::from_state_id(state.id).facing,
                );
                world
                    .get_block_state(&pos.offset(facing.opposite().to_offset()))
                    .is_side_solid(facing)
            }
            // Solid feature blocks do not require plant support.
            _ => true,
        }
    }

    fn mirror(&self, block: &Block, state_id: BlockStateId, mirror: Mirror) -> &'static BlockState {
        block.mirror(state_id, mirror)
    }

    fn rotate(
        &self,
        block: &Block,
        state_id: BlockStateId,
        rotation: Rotation,
    ) -> &'static BlockState {
        block.rotate(state_id, rotation)
    }
}
