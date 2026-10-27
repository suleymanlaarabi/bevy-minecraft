//! Native block identities and shared registry metadata.
use pumpkin_data::{Block, BlockId};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VoxelKind(pub u32);

impl Default for VoxelKind {
    fn default() -> Self {
        Self::Air
    }
}

// Per-block gameplay tuning; identities and names come from the native registry.
macro_rules! block_tuning {
    ($($kind:ident: $native:ident, $fallback:ident, $color:expr;)*) => {
        #[allow(non_upper_case_globals)]
        impl VoxelKind {
            $(pub const $kind: Self = Self(BlockId::$native.as_u16() as u32);)*
            const fn tuned_fallback(self) -> Option<Self> {
                match self {$(Self::$kind => Some(Self::$fallback),)* _ => None}
            }
            pub const fn color(self) -> [f32;4] {
                match self {$(Self::$kind => $color,)* _ => [1.0;4]}
            }
        }
    };
}
block_tuning! {
    Air: AIR, Stone, [0.0, 0.0, 0.0, 0.0];
    Grass: GRASS_BLOCK, Stone, [0.25, 0.65, 0.18, 1.0];
    Dirt: DIRT, Stone, [0.40, 0.25, 0.12, 1.0];
    Stone: STONE, Stone, [0.40, 0.40, 0.40, 1.0];
    Sand: SAND, Stone, [0.76, 0.68, 0.42, 1.0];
    Snow: SNOW, Stone, [0.92, 0.96, 1.0, 1.0];
    Wood: OAK_LOG, Stone, [0.36, 0.20, 0.08, 1.0];
    Leaves: OAK_LEAVES, Stone, [0.12, 0.48, 0.10, 1.0];
    Deepslate: DEEPSLATE, Stone, [0.22, 0.22, 0.25, 1.0];
    Gravel: GRAVEL, Stone, [0.48, 0.46, 0.43, 1.0];
    CoalOre: COAL_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    IronOre: IRON_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    CopperOre: COPPER_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    GoldOre: GOLD_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    RedstoneOre: REDSTONE_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    DiamondOre: DIAMOND_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    EmeraldOre: EMERALD_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    LapisOre: LAPIS_ORE, Stone, [0.48, 0.46, 0.43, 1.0];
    DeepslateCoalOre: DEEPSLATE_COAL_ORE, CoalOre, [0.25, 0.25, 0.27, 1.0];
    DeepslateIronOre: DEEPSLATE_IRON_ORE, IronOre, [0.25, 0.25, 0.27, 1.0];
    DeepslateCopperOre: DEEPSLATE_COPPER_ORE, CopperOre, [0.25, 0.25, 0.27, 1.0];
    DeepslateGoldOre: DEEPSLATE_GOLD_ORE, GoldOre, [0.25, 0.25, 0.27, 1.0];
    DeepslateRedstoneOre: DEEPSLATE_REDSTONE_ORE, RedstoneOre, [0.25, 0.25, 0.27, 1.0];
    DeepslateDiamondOre: DEEPSLATE_DIAMOND_ORE, DiamondOre, [0.25, 0.25, 0.27, 1.0];
    DeepslateEmeraldOre: DEEPSLATE_EMERALD_ORE, EmeraldOre, [0.25, 0.25, 0.27, 1.0];
    DeepslateLapisOre: DEEPSLATE_LAPIS_ORE, LapisOre, [0.25, 0.25, 0.27, 1.0];
    Granite: GRANITE, Stone, [0.60, 0.42, 0.35, 1.0];
    Diorite: DIORITE, Stone, [0.75, 0.75, 0.75, 1.0];
    Andesite: ANDESITE, Stone, [0.53, 0.53, 0.53, 1.0];
    Tuff: TUFF, Stone, [0.42, 0.43, 0.38, 1.0];
    Calcite: CALCITE, Stone, [0.87, 0.88, 0.86, 1.0];
    Bedrock: BEDROCK, Stone, [0.33, 0.33, 0.33, 1.0];
    Cobblestone: COBBLESTONE, Stone, [0.48, 0.48, 0.48, 1.0];
    MossyCobblestone: MOSSY_COBBLESTONE, Cobblestone, [0.43, 0.48, 0.38, 1.0];
    CobbledDeepslate: COBBLED_DEEPSLATE, Deepslate, [0.30, 0.30, 0.32, 1.0];
    StoneBricks: STONE_BRICKS, Stone, [0.48, 0.48, 0.48, 1.0];
    MossyStoneBricks: MOSSY_STONE_BRICKS, StoneBricks, [0.43, 0.48, 0.38, 1.0];
    CrackedStoneBricks: CRACKED_STONE_BRICKS, StoneBricks, [0.46, 0.46, 0.46, 1.0];
    Obsidian: OBSIDIAN, Stone, [0.08, 0.06, 0.13, 1.0];
    SmoothBasalt: SMOOTH_BASALT, Stone, [0.28, 0.28, 0.31, 1.0];
    DripstoneBlock: DRIPSTONE_BLOCK, Stone, [0.53, 0.42, 0.36, 1.0];
    AmethystBlock: AMETHYST_BLOCK, Stone, [0.52, 0.38, 0.75, 1.0];
    BuddingAmethyst: BUDDING_AMETHYST, AmethystBlock, [0.52, 0.38, 0.75, 1.0];
    MagmaBlock: MAGMA_BLOCK, Stone, [0.55, 0.25, 0.10, 1.0];
    Lava: LAVA, Stone, [0.85, 0.40, 0.08, 1.0];
    CoarseDirt: COARSE_DIRT, Dirt, [0.45, 0.32, 0.22, 1.0];
    RootedDirt: ROOTED_DIRT, Dirt, [0.55, 0.40, 0.30, 1.0];
    Podzol: PODZOL, Dirt, [0.36, 0.25, 0.10, 1.0];
    Mycelium: MYCELIUM, Grass, [0.44, 0.38, 0.42, 1.0];
    Mud: MUD, Dirt, [0.24, 0.22, 0.24, 1.0];
    Clay: CLAY, Dirt, [0.62, 0.64, 0.70, 1.0];
    MossBlock: MOSS_BLOCK, Grass, [0.35, 0.43, 0.18, 1.0];
    DirtPath: DIRT_PATH, Dirt, [0.58, 0.48, 0.25, 1.0];
    Farmland: FARMLAND, Dirt, [0.32, 0.20, 0.10, 1.0];
    RedSand: RED_SAND, Sand, [0.75, 0.40, 0.13, 1.0];
    Sandstone: SANDSTONE, Sand, [0.85, 0.80, 0.60, 1.0];
    RedSandstone: RED_SANDSTONE, Sandstone, [0.72, 0.38, 0.12, 1.0];
    Terracotta: TERRACOTTA, Stone, [0.60, 0.37, 0.27, 1.0];
    WhiteTerracotta: WHITE_TERRACOTTA, Terracotta, [0.82, 0.70, 0.63, 1.0];
    OrangeTerracotta: ORANGE_TERRACOTTA, Terracotta, [0.63, 0.33, 0.15, 1.0];
    YellowTerracotta: YELLOW_TERRACOTTA, Terracotta, [0.73, 0.52, 0.21, 1.0];
    BrownTerracotta: BROWN_TERRACOTTA, Terracotta, [0.30, 0.20, 0.14, 1.0];
    RedTerracotta: RED_TERRACOTTA, Terracotta, [0.56, 0.24, 0.18, 1.0];
    LightGrayTerracotta: LIGHT_GRAY_TERRACOTTA, Terracotta, [0.53, 0.42, 0.38, 1.0];
    SnowBlock: SNOW_BLOCK, Snow, [0.95, 0.98, 1.0, 1.0];
    PowderSnow: POWDER_SNOW, Snow, [0.97, 0.99, 1.0, 1.0];
    Ice: ICE, Snow, [0.57, 0.72, 0.98, 1.0];
    PackedIce: PACKED_ICE, Ice, [0.55, 0.68, 0.95, 1.0];
    BlueIce: BLUE_ICE, Ice, [0.45, 0.62, 0.98, 1.0];
    SpruceLog: SPRUCE_LOG, Wood, [0.23, 0.15, 0.08, 1.0];
    BirchLog: BIRCH_LOG, Wood, [0.85, 0.83, 0.78, 1.0];
    JungleLog: JUNGLE_LOG, Wood, [0.33, 0.26, 0.10, 1.0];
    AcaciaLog: ACACIA_LOG, Wood, [0.40, 0.38, 0.35, 1.0];
    DarkOakLog: DARK_OAK_LOG, Wood, [0.24, 0.18, 0.10, 1.0];
    MangroveLog: MANGROVE_LOG, Wood, [0.33, 0.26, 0.17, 1.0];
    CherryLog: CHERRY_LOG, Wood, [0.21, 0.13, 0.16, 1.0];
    PaleOakLog: PALE_OAK_LOG, Wood, [0.38, 0.35, 0.33, 1.0];
    SpruceLeaves: SPRUCE_LEAVES, Leaves, [0.24, 0.38, 0.24, 1.0];
    BirchLeaves: BIRCH_LEAVES, Leaves, [0.36, 0.50, 0.25, 1.0];
    JungleLeaves: JUNGLE_LEAVES, Leaves, [0.20, 0.55, 0.10, 1.0];
    AcaciaLeaves: ACACIA_LEAVES, Leaves, [0.30, 0.50, 0.12, 1.0];
    DarkOakLeaves: DARK_OAK_LEAVES, Leaves, [0.15, 0.40, 0.08, 1.0];
    MangroveLeaves: MANGROVE_LEAVES, Leaves, [0.30, 0.50, 0.12, 1.0];
    CherryLeaves: CHERRY_LEAVES, Leaves, [0.90, 0.65, 0.80, 1.0];
    PaleOakLeaves: PALE_OAK_LEAVES, Leaves, [0.60, 0.62, 0.58, 1.0];
    AzaleaLeaves: AZALEA_LEAVES, Leaves, [0.35, 0.48, 0.20, 1.0];
    FloweringAzaleaLeaves: FLOWERING_AZALEA_LEAVES, AzaleaLeaves, [0.45, 0.48, 0.30, 1.0];
    OakPlanks: OAK_PLANKS, Wood, [0.64, 0.51, 0.31, 1.0];
    SprucePlanks: SPRUCE_PLANKS, OakPlanks, [0.45, 0.33, 0.20, 1.0];
    BirchPlanks: BIRCH_PLANKS, OakPlanks, [0.76, 0.69, 0.48, 1.0];
    JunglePlanks: JUNGLE_PLANKS, OakPlanks, [0.63, 0.45, 0.31, 1.0];
    AcaciaPlanks: ACACIA_PLANKS, OakPlanks, [0.66, 0.36, 0.20, 1.0];
    DarkOakPlanks: DARK_OAK_PLANKS, OakPlanks, [0.26, 0.17, 0.08, 1.0];
    Bricks: BRICKS, Stone, [0.59, 0.38, 0.32, 1.0];
    Prismarine: PRISMARINE, Stone, [0.39, 0.62, 0.57, 1.0];
    PrismarineBricks: PRISMARINE_BRICKS, Prismarine, [0.39, 0.67, 0.59, 1.0];
    DarkPrismarine: DARK_PRISMARINE, Prismarine, [0.20, 0.36, 0.30, 1.0];
    SeaLantern: SEA_LANTERN, Prismarine, [0.80, 0.87, 0.85, 1.0];
    BrownMushroomBlock: BROWN_MUSHROOM_BLOCK, Wood, [0.58, 0.44, 0.33, 1.0];
    RedMushroomBlock: RED_MUSHROOM_BLOCK, Wood, [0.78, 0.18, 0.17, 1.0];
    MushroomStem: MUSHROOM_STEM, Wood, [0.80, 0.78, 0.72, 1.0];
    Pumpkin: PUMPKIN, Wood, [0.78, 0.45, 0.10, 1.0];
    Melon: MELON, Wood, [0.45, 0.58, 0.15, 1.0];
    HayBlock: HAY_BLOCK, Wood, [0.65, 0.55, 0.12, 1.0];
    Bookshelf: BOOKSHELF, OakPlanks, [0.50, 0.40, 0.28, 1.0];
    Water: WATER, Stone, [0.08, 0.35, 0.72, 0.62];
}

impl VoxelKind {
    pub const ALL: [Self; BlockId::COUNT as usize] = {
        let mut kinds = [Self::Air; BlockId::COUNT as usize];
        let mut i = 0;
        while i < kinds.len() {
            kinds[i] = Self(i as u32);
            i += 1;
        }
        kinds
    };

    pub const fn index(self) -> usize {
        self.0 as usize
    }
    pub fn block(self) -> &'static Block {
        self.native_id().expect("Native-only metadata").to_block()
    }
    pub fn name(self) -> &'static str {
        if self.is_native() {
            self.block().name
        } else {
            &super::content::catalog()
                .block(hollow_core::BlockId(self.0))
                .name
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        Block::from_registry_key(key.strip_prefix("minecraft:").unwrap_or(key))
            .map(|block| Self::from_native(block.id))
            .or_else(|| {
                super::content::try_catalog()?
                    .resolve(key)
                    .map(|id| Self(id.0))
            })
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

    pub fn is_itemizable(self) -> bool {
        self.is_native()
            && self.block().item_id != 0
            && !matches!(self, Self::Air | Self::Water | Self::Lava)
    }
    pub fn is_solid(self) -> bool {
        if self.is_native() {
            !self.block().default_state.collision_shapes.is_empty()
        } else {
            !self.default_state().collision_empty()
        }
    }
    pub fn is_breakable(self) -> bool {
        self.default_state().definition().hardness >= 0.0
            && !self.default_state().is_air()
            && !self.is_liquid()
    }
    pub fn is_liquid(self) -> bool {
        self.default_state().is_liquid()
    }
    pub fn is_opaque(self) -> bool {
        self.default_state().is_opaque_cube()
    }

    pub fn fallback(self) -> Self {
        if !self.is_native() {
            return self;
        }
        if let Some(fallback) = self.tuned_fallback() {
            return fallback;
        }
        if let Some(id) = super::block_relations::material(self.native_id().unwrap()) {
            return Self::from_native(id);
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

    pub fn tagged(self, tag: pumpkin_data::tag::Tag) -> bool {
        self.native_id().is_some_and(|id| id.has_tag(tag))
    }
}

impl VoxelKind {
    pub const fn from_native(id: BlockId) -> Self {
        Self(id.as_u16() as u32)
    }
    pub const fn is_native(self) -> bool {
        self.0 < BlockId::COUNT as u32
    }
    pub fn native_id(self) -> Option<BlockId> {
        self.is_native().then(|| BlockId::new_or_air(self.0 as u16))
    }
    pub fn count() -> usize {
        super::content::try_catalog().map_or(BlockId::COUNT as usize, |c| c.block_count())
    }
    pub fn all() -> impl Iterator<Item = Self> {
        (0..Self::count()).map(|id| Self(id as u32))
    }
    pub fn physics(self) -> &'static hollow_core::content::BlockDefinition {
        super::content::catalog().block(hollow_core::BlockId(self.0))
    }
}
const _: () = assert!(std::mem::size_of::<VoxelKind>() == 4);
