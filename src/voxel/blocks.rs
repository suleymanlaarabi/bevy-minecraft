//! Native block identities and shared registry metadata.
use pumpkin_data::{Block, BlockId};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VoxelKind(pub(crate) BlockId);

impl Default for VoxelKind {
    fn default() -> Self {
        Self::Air
    }
}

// Per-block gameplay tuning; identities and names come from the native registry.
macro_rules! block_tuning {
    ($($kind:ident: $native:ident, $fallback:ident, $duration:expr, $color:expr;)*) => {
        #[allow(non_upper_case_globals)]
        impl VoxelKind {
            $(pub const $kind: Self = Self(BlockId::$native);)*
            const fn tuned_fallback(self) -> Option<Self> {
                match self {$(Self::$kind => Some(Self::$fallback),)* _ => None}
            }
            pub const fn break_duration(self) -> Option<f32> {
                match self {$(Self::$kind => $duration,)* _ => {
                    let hardness=self.block().hardness;
                    if hardness<0.0 {None}else{Some(hardness)}
                }}
            }
            pub(crate) const fn color(self) -> [f32;4] {
                match self {$(Self::$kind => $color,)* _ => [1.0;4]}
            }
        }
    };
}
block_tuning! {
    Air: AIR, Stone, None, [0.0, 0.0, 0.0, 0.0];
    Grass: GRASS_BLOCK, Stone, Some(0.6), [0.25, 0.65, 0.18, 1.0];
    Dirt: DIRT, Stone, Some(0.5), [0.40, 0.25, 0.12, 1.0];
    Stone: STONE, Stone, Some(1.5), [0.40, 0.40, 0.40, 1.0];
    Sand: SAND, Stone, Some(0.5), [0.76, 0.68, 0.42, 1.0];
    Snow: SNOW, Stone, Some(0.3), [0.92, 0.96, 1.0, 1.0];
    Wood: OAK_LOG, Stone, Some(1.2), [0.36, 0.20, 0.08, 1.0];
    Leaves: OAK_LEAVES, Stone, Some(0.25), [0.12, 0.48, 0.10, 1.0];
    Deepslate: DEEPSLATE, Stone, Some(2.0), [0.22, 0.22, 0.25, 1.0];
    Gravel: GRAVEL, Stone, Some(0.6), [0.48, 0.46, 0.43, 1.0];
    CoalOre: COAL_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    IronOre: IRON_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    CopperOre: COPPER_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    GoldOre: GOLD_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    RedstoneOre: REDSTONE_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    DiamondOre: DIAMOND_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    EmeraldOre: EMERALD_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    LapisOre: LAPIS_ORE, Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    DeepslateCoalOre: DEEPSLATE_COAL_ORE, CoalOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateIronOre: DEEPSLATE_IRON_ORE, IronOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateCopperOre: DEEPSLATE_COPPER_ORE, CopperOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateGoldOre: DEEPSLATE_GOLD_ORE, GoldOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateRedstoneOre: DEEPSLATE_REDSTONE_ORE, RedstoneOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateDiamondOre: DEEPSLATE_DIAMOND_ORE, DiamondOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateEmeraldOre: DEEPSLATE_EMERALD_ORE, EmeraldOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateLapisOre: DEEPSLATE_LAPIS_ORE, LapisOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    Granite: GRANITE, Stone, Some(1.5), [0.60, 0.42, 0.35, 1.0];
    Diorite: DIORITE, Stone, Some(1.5), [0.75, 0.75, 0.75, 1.0];
    Andesite: ANDESITE, Stone, Some(1.5), [0.53, 0.53, 0.53, 1.0];
    Tuff: TUFF, Stone, Some(1.5), [0.42, 0.43, 0.38, 1.0];
    Calcite: CALCITE, Stone, Some(0.8), [0.87, 0.88, 0.86, 1.0];
    Bedrock: BEDROCK, Stone, None, [0.33, 0.33, 0.33, 1.0];
    Cobblestone: COBBLESTONE, Stone, Some(2.0), [0.48, 0.48, 0.48, 1.0];
    MossyCobblestone: MOSSY_COBBLESTONE, Cobblestone, Some(2.0), [0.43, 0.48, 0.38, 1.0];
    CobbledDeepslate: COBBLED_DEEPSLATE, Deepslate, Some(3.5), [0.30, 0.30, 0.32, 1.0];
    StoneBricks: STONE_BRICKS, Stone, Some(1.5), [0.48, 0.48, 0.48, 1.0];
    MossyStoneBricks: MOSSY_STONE_BRICKS, StoneBricks, Some(1.5), [0.43, 0.48, 0.38, 1.0];
    CrackedStoneBricks: CRACKED_STONE_BRICKS, StoneBricks, Some(1.5), [0.46, 0.46, 0.46, 1.0];
    Obsidian: OBSIDIAN, Stone, Some(10.0), [0.08, 0.06, 0.13, 1.0];
    SmoothBasalt: SMOOTH_BASALT, Stone, Some(1.25), [0.28, 0.28, 0.31, 1.0];
    DripstoneBlock: DRIPSTONE_BLOCK, Stone, Some(1.5), [0.53, 0.42, 0.36, 1.0];
    AmethystBlock: AMETHYST_BLOCK, Stone, Some(1.5), [0.52, 0.38, 0.75, 1.0];
    BuddingAmethyst: BUDDING_AMETHYST, AmethystBlock, Some(1.5), [0.52, 0.38, 0.75, 1.0];
    MagmaBlock: MAGMA_BLOCK, Stone, Some(0.5), [0.55, 0.25, 0.10, 1.0];
    Lava: LAVA, Stone, None, [0.85, 0.40, 0.08, 1.0];
    CoarseDirt: COARSE_DIRT, Dirt, Some(0.5), [0.45, 0.32, 0.22, 1.0];
    RootedDirt: ROOTED_DIRT, Dirt, Some(0.5), [0.55, 0.40, 0.30, 1.0];
    Podzol: PODZOL, Dirt, Some(0.5), [0.36, 0.25, 0.10, 1.0];
    Mycelium: MYCELIUM, Grass, Some(0.6), [0.44, 0.38, 0.42, 1.0];
    Mud: MUD, Dirt, Some(0.5), [0.24, 0.22, 0.24, 1.0];
    Clay: CLAY, Dirt, Some(0.6), [0.62, 0.64, 0.70, 1.0];
    MossBlock: MOSS_BLOCK, Grass, Some(0.1), [0.35, 0.43, 0.18, 1.0];
    DirtPath: DIRT_PATH, Dirt, Some(0.65), [0.58, 0.48, 0.25, 1.0];
    Farmland: FARMLAND, Dirt, Some(0.6), [0.32, 0.20, 0.10, 1.0];
    RedSand: RED_SAND, Sand, Some(0.5), [0.75, 0.40, 0.13, 1.0];
    Sandstone: SANDSTONE, Sand, Some(0.8), [0.85, 0.80, 0.60, 1.0];
    RedSandstone: RED_SANDSTONE, Sandstone, Some(0.8), [0.72, 0.38, 0.12, 1.0];
    Terracotta: TERRACOTTA, Stone, Some(1.25), [0.60, 0.37, 0.27, 1.0];
    WhiteTerracotta: WHITE_TERRACOTTA, Terracotta, Some(1.25), [0.82, 0.70, 0.63, 1.0];
    OrangeTerracotta: ORANGE_TERRACOTTA, Terracotta, Some(1.25), [0.63, 0.33, 0.15, 1.0];
    YellowTerracotta: YELLOW_TERRACOTTA, Terracotta, Some(1.25), [0.73, 0.52, 0.21, 1.0];
    BrownTerracotta: BROWN_TERRACOTTA, Terracotta, Some(1.25), [0.30, 0.20, 0.14, 1.0];
    RedTerracotta: RED_TERRACOTTA, Terracotta, Some(1.25), [0.56, 0.24, 0.18, 1.0];
    LightGrayTerracotta: LIGHT_GRAY_TERRACOTTA, Terracotta, Some(1.25), [0.53, 0.42, 0.38, 1.0];
    SnowBlock: SNOW_BLOCK, Snow, Some(0.2), [0.95, 0.98, 1.0, 1.0];
    PowderSnow: POWDER_SNOW, Snow, Some(0.25), [0.97, 0.99, 1.0, 1.0];
    Ice: ICE, Snow, Some(0.5), [0.57, 0.72, 0.98, 1.0];
    PackedIce: PACKED_ICE, Ice, Some(0.5), [0.55, 0.68, 0.95, 1.0];
    BlueIce: BLUE_ICE, Ice, Some(2.8), [0.45, 0.62, 0.98, 1.0];
    SpruceLog: SPRUCE_LOG, Wood, Some(1.2), [0.23, 0.15, 0.08, 1.0];
    BirchLog: BIRCH_LOG, Wood, Some(1.2), [0.85, 0.83, 0.78, 1.0];
    JungleLog: JUNGLE_LOG, Wood, Some(1.2), [0.33, 0.26, 0.10, 1.0];
    AcaciaLog: ACACIA_LOG, Wood, Some(1.2), [0.40, 0.38, 0.35, 1.0];
    DarkOakLog: DARK_OAK_LOG, Wood, Some(1.2), [0.24, 0.18, 0.10, 1.0];
    MangroveLog: MANGROVE_LOG, Wood, Some(1.2), [0.33, 0.26, 0.17, 1.0];
    CherryLog: CHERRY_LOG, Wood, Some(1.2), [0.21, 0.13, 0.16, 1.0];
    PaleOakLog: PALE_OAK_LOG, Wood, Some(1.2), [0.38, 0.35, 0.33, 1.0];
    SpruceLeaves: SPRUCE_LEAVES, Leaves, Some(0.25), [0.24, 0.38, 0.24, 1.0];
    BirchLeaves: BIRCH_LEAVES, Leaves, Some(0.25), [0.36, 0.50, 0.25, 1.0];
    JungleLeaves: JUNGLE_LEAVES, Leaves, Some(0.25), [0.20, 0.55, 0.10, 1.0];
    AcaciaLeaves: ACACIA_LEAVES, Leaves, Some(0.25), [0.30, 0.50, 0.12, 1.0];
    DarkOakLeaves: DARK_OAK_LEAVES, Leaves, Some(0.25), [0.15, 0.40, 0.08, 1.0];
    MangroveLeaves: MANGROVE_LEAVES, Leaves, Some(0.25), [0.30, 0.50, 0.12, 1.0];
    CherryLeaves: CHERRY_LEAVES, Leaves, Some(0.25), [0.90, 0.65, 0.80, 1.0];
    PaleOakLeaves: PALE_OAK_LEAVES, Leaves, Some(0.25), [0.60, 0.62, 0.58, 1.0];
    AzaleaLeaves: AZALEA_LEAVES, Leaves, Some(0.25), [0.35, 0.48, 0.20, 1.0];
    FloweringAzaleaLeaves: FLOWERING_AZALEA_LEAVES, AzaleaLeaves, Some(0.25), [0.45, 0.48, 0.30, 1.0];
    OakPlanks: OAK_PLANKS, Wood, Some(1.0), [0.64, 0.51, 0.31, 1.0];
    SprucePlanks: SPRUCE_PLANKS, OakPlanks, Some(1.0), [0.45, 0.33, 0.20, 1.0];
    BirchPlanks: BIRCH_PLANKS, OakPlanks, Some(1.0), [0.76, 0.69, 0.48, 1.0];
    JunglePlanks: JUNGLE_PLANKS, OakPlanks, Some(1.0), [0.63, 0.45, 0.31, 1.0];
    AcaciaPlanks: ACACIA_PLANKS, OakPlanks, Some(1.0), [0.66, 0.36, 0.20, 1.0];
    DarkOakPlanks: DARK_OAK_PLANKS, OakPlanks, Some(1.0), [0.26, 0.17, 0.08, 1.0];
    Bricks: BRICKS, Stone, Some(2.0), [0.59, 0.38, 0.32, 1.0];
    Prismarine: PRISMARINE, Stone, Some(1.5), [0.39, 0.62, 0.57, 1.0];
    PrismarineBricks: PRISMARINE_BRICKS, Prismarine, Some(1.5), [0.39, 0.67, 0.59, 1.0];
    DarkPrismarine: DARK_PRISMARINE, Prismarine, Some(1.5), [0.20, 0.36, 0.30, 1.0];
    SeaLantern: SEA_LANTERN, Prismarine, Some(0.3), [0.80, 0.87, 0.85, 1.0];
    BrownMushroomBlock: BROWN_MUSHROOM_BLOCK, Wood, Some(0.2), [0.58, 0.44, 0.33, 1.0];
    RedMushroomBlock: RED_MUSHROOM_BLOCK, Wood, Some(0.2), [0.78, 0.18, 0.17, 1.0];
    MushroomStem: MUSHROOM_STEM, Wood, Some(0.2), [0.80, 0.78, 0.72, 1.0];
    Pumpkin: PUMPKIN, Wood, Some(1.0), [0.78, 0.45, 0.10, 1.0];
    Melon: MELON, Wood, Some(1.0), [0.45, 0.58, 0.15, 1.0];
    HayBlock: HAY_BLOCK, Wood, Some(0.5), [0.65, 0.55, 0.12, 1.0];
    Bookshelf: BOOKSHELF, OakPlanks, Some(1.5), [0.50, 0.40, 0.28, 1.0];
    Water: WATER, Stone, None, [0.08, 0.35, 0.72, 0.62];
}

impl VoxelKind {
    pub const ALL: [Self; BlockId::COUNT as usize] = {
        let mut kinds = [Self::Air; BlockId::COUNT as usize];
        let mut i = 0;
        while i < kinds.len() {
            kinds[i] = Self(BlockId::new_or_air(i as u16));
            i += 1;
        }
        kinds
    };

    pub const fn index(self) -> usize {
        self.0.as_u16() as usize
    }
    pub(crate) const fn block(self) -> &'static Block {
        self.0.to_block()
    }
    pub const fn name(self) -> &'static str {
        self.block().name
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Block::from_registry_key(key.strip_prefix("minecraft:").unwrap_or(key))
            .map(|block| Self(block.id))
    }

    pub fn itemizable() -> &'static [Self] {
        static ITEMS: OnceLock<Box<[VoxelKind]>> = OnceLock::new();
        ITEMS.get_or_init(|| {
            Self::ALL
                .into_iter()
                .filter(|kind| kind.is_itemizable())
                .collect()
        })
    }

    pub const fn is_itemizable(self) -> bool {
        self.block().item_id != 0 && !matches!(self, Self::Air | Self::Water | Self::Lava)
    }
    pub const fn is_solid(self) -> bool {
        !self.block().default_state.collision_shapes.is_empty()
    }
    pub const fn is_liquid(self) -> bool {
        matches!(self, Self::Water | Self::Lava)
    }
    pub(crate) const fn is_opaque(self) -> bool {
        self.block().default_state.is_full_cube() && self.block().default_state.opacity == 15
    }

    pub(crate) fn fallback(self) -> Self {
        if let Some(fallback) = self.tuned_fallback() {
            return fallback;
        }
        if let Some(id) = super::block_relations::material(self.0) {
            return Self(id);
        }
        if self.tagged(pumpkin_data::tag::Block::MINECRAFT_LEAVES) || !self.is_solid() {
            Self::Leaves
        } else if self.tagged(pumpkin_data::tag::Block::C_GLASS_BLOCKS)
            || self.tagged(pumpkin_data::tag::Block::C_GLASS_PANES)
        {
            Self::Ice
        } else {
            Self::Stone
        }
    }

    pub(crate) fn tagged(self, tag: pumpkin_data::tag::Tag) -> bool {
        self.0.has_tag(tag)
    }
}

const _: () = assert!(std::mem::size_of::<VoxelKind>() == 2);
