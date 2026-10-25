//! Native block identities. States, shapes and light remain separate section layers.
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

// Legacy names and tuning remain in one table; all other identities come from Pumpkin.
macro_rules! legacy_kinds {
    ($($kind:ident: $native:ident, $key:literal, $fallback:ident, $duration:expr, $color:expr;)*) => {
        #[allow(non_upper_case_globals)]
        impl VoxelKind {
            $(pub const $kind: Self = Self(BlockId::$native);)*
            pub const fn key(self) -> &'static str {
                match self {$(Self::$kind => $key,)* _ => self.name()}
            }
            const fn legacy_fallback(self) -> Option<Self> {
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
legacy_kinds! {
    Air: AIR, "air", Stone, None, [0.0, 0.0, 0.0, 0.0];
    Grass: GRASS_BLOCK, "grass", Stone, Some(0.6), [0.25, 0.65, 0.18, 1.0];
    Dirt: DIRT, "dirt", Stone, Some(0.5), [0.40, 0.25, 0.12, 1.0];
    Stone: STONE, "stone", Stone, Some(1.5), [0.40, 0.40, 0.40, 1.0];
    Sand: SAND, "sand", Stone, Some(0.5), [0.76, 0.68, 0.42, 1.0];
    Snow: SNOW, "snow", Stone, Some(0.3), [0.92, 0.96, 1.0, 1.0];
    Wood: OAK_LOG, "wood", Stone, Some(1.2), [0.36, 0.20, 0.08, 1.0];
    Leaves: OAK_LEAVES, "leaves", Stone, Some(0.25), [0.12, 0.48, 0.10, 1.0];
    Deepslate: DEEPSLATE, "deepslate", Stone, Some(2.0), [0.22, 0.22, 0.25, 1.0];
    Gravel: GRAVEL, "gravel", Stone, Some(0.6), [0.48, 0.46, 0.43, 1.0];
    CoalOre: COAL_ORE, "coal_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    IronOre: IRON_ORE, "iron_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    CopperOre: COPPER_ORE, "copper_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    GoldOre: GOLD_ORE, "gold_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    RedstoneOre: REDSTONE_ORE, "redstone_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    DiamondOre: DIAMOND_ORE, "diamond_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    EmeraldOre: EMERALD_ORE, "emerald_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    LapisOre: LAPIS_ORE, "lapis_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    DeepslateCoalOre: DEEPSLATE_COAL_ORE, "deepslate_coal_ore", CoalOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateIronOre: DEEPSLATE_IRON_ORE, "deepslate_iron_ore", IronOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateCopperOre: DEEPSLATE_COPPER_ORE, "deepslate_copper_ore", CopperOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateGoldOre: DEEPSLATE_GOLD_ORE, "deepslate_gold_ore", GoldOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateRedstoneOre: DEEPSLATE_REDSTONE_ORE, "deepslate_redstone_ore", RedstoneOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateDiamondOre: DEEPSLATE_DIAMOND_ORE, "deepslate_diamond_ore", DiamondOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateEmeraldOre: DEEPSLATE_EMERALD_ORE, "deepslate_emerald_ore", EmeraldOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateLapisOre: DEEPSLATE_LAPIS_ORE, "deepslate_lapis_ore", LapisOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    Granite: GRANITE, "granite", Stone, Some(1.5), [0.60, 0.42, 0.35, 1.0];
    Diorite: DIORITE, "diorite", Stone, Some(1.5), [0.75, 0.75, 0.75, 1.0];
    Andesite: ANDESITE, "andesite", Stone, Some(1.5), [0.53, 0.53, 0.53, 1.0];
    Tuff: TUFF, "tuff", Stone, Some(1.5), [0.42, 0.43, 0.38, 1.0];
    Calcite: CALCITE, "calcite", Stone, Some(0.8), [0.87, 0.88, 0.86, 1.0];
    Bedrock: BEDROCK, "bedrock", Stone, None, [0.33, 0.33, 0.33, 1.0];
    Cobblestone: COBBLESTONE, "cobblestone", Stone, Some(2.0), [0.48, 0.48, 0.48, 1.0];
    MossyCobblestone: MOSSY_COBBLESTONE, "mossy_cobblestone", Cobblestone, Some(2.0), [0.43, 0.48, 0.38, 1.0];
    CobbledDeepslate: COBBLED_DEEPSLATE, "cobbled_deepslate", Deepslate, Some(3.5), [0.30, 0.30, 0.32, 1.0];
    StoneBricks: STONE_BRICKS, "stone_bricks", Stone, Some(1.5), [0.48, 0.48, 0.48, 1.0];
    MossyStoneBricks: MOSSY_STONE_BRICKS, "mossy_stone_bricks", StoneBricks, Some(1.5), [0.43, 0.48, 0.38, 1.0];
    CrackedStoneBricks: CRACKED_STONE_BRICKS, "cracked_stone_bricks", StoneBricks, Some(1.5), [0.46, 0.46, 0.46, 1.0];
    Obsidian: OBSIDIAN, "obsidian", Stone, Some(10.0), [0.08, 0.06, 0.13, 1.0];
    SmoothBasalt: SMOOTH_BASALT, "smooth_basalt", Stone, Some(1.25), [0.28, 0.28, 0.31, 1.0];
    DripstoneBlock: DRIPSTONE_BLOCK, "dripstone_block", Stone, Some(1.5), [0.53, 0.42, 0.36, 1.0];
    AmethystBlock: AMETHYST_BLOCK, "amethyst_block", Stone, Some(1.5), [0.52, 0.38, 0.75, 1.0];
    BuddingAmethyst: BUDDING_AMETHYST, "budding_amethyst", AmethystBlock, Some(1.5), [0.52, 0.38, 0.75, 1.0];
    MagmaBlock: MAGMA_BLOCK, "magma_block", Stone, Some(0.5), [0.55, 0.25, 0.10, 1.0];
    Lava: LAVA, "lava", Stone, None, [0.85, 0.40, 0.08, 1.0];
    CoarseDirt: COARSE_DIRT, "coarse_dirt", Dirt, Some(0.5), [0.45, 0.32, 0.22, 1.0];
    RootedDirt: ROOTED_DIRT, "rooted_dirt", Dirt, Some(0.5), [0.55, 0.40, 0.30, 1.0];
    Podzol: PODZOL, "podzol", Dirt, Some(0.5), [0.36, 0.25, 0.10, 1.0];
    Mycelium: MYCELIUM, "mycelium", Grass, Some(0.6), [0.44, 0.38, 0.42, 1.0];
    Mud: MUD, "mud", Dirt, Some(0.5), [0.24, 0.22, 0.24, 1.0];
    Clay: CLAY, "clay", Dirt, Some(0.6), [0.62, 0.64, 0.70, 1.0];
    MossBlock: MOSS_BLOCK, "moss_block", Grass, Some(0.1), [0.35, 0.43, 0.18, 1.0];
    DirtPath: DIRT_PATH, "dirt_path", Dirt, Some(0.65), [0.58, 0.48, 0.25, 1.0];
    Farmland: FARMLAND, "farmland", Dirt, Some(0.6), [0.32, 0.20, 0.10, 1.0];
    RedSand: RED_SAND, "red_sand", Sand, Some(0.5), [0.75, 0.40, 0.13, 1.0];
    Sandstone: SANDSTONE, "sandstone", Sand, Some(0.8), [0.85, 0.80, 0.60, 1.0];
    RedSandstone: RED_SANDSTONE, "red_sandstone", Sandstone, Some(0.8), [0.72, 0.38, 0.12, 1.0];
    Terracotta: TERRACOTTA, "terracotta", Stone, Some(1.25), [0.60, 0.37, 0.27, 1.0];
    WhiteTerracotta: WHITE_TERRACOTTA, "white_terracotta", Terracotta, Some(1.25), [0.82, 0.70, 0.63, 1.0];
    OrangeTerracotta: ORANGE_TERRACOTTA, "orange_terracotta", Terracotta, Some(1.25), [0.63, 0.33, 0.15, 1.0];
    YellowTerracotta: YELLOW_TERRACOTTA, "yellow_terracotta", Terracotta, Some(1.25), [0.73, 0.52, 0.21, 1.0];
    BrownTerracotta: BROWN_TERRACOTTA, "brown_terracotta", Terracotta, Some(1.25), [0.30, 0.20, 0.14, 1.0];
    RedTerracotta: RED_TERRACOTTA, "red_terracotta", Terracotta, Some(1.25), [0.56, 0.24, 0.18, 1.0];
    LightGrayTerracotta: LIGHT_GRAY_TERRACOTTA, "light_gray_terracotta", Terracotta, Some(1.25), [0.53, 0.42, 0.38, 1.0];
    SnowBlock: SNOW_BLOCK, "snow_block", Snow, Some(0.2), [0.95, 0.98, 1.0, 1.0];
    PowderSnow: POWDER_SNOW, "powder_snow", Snow, Some(0.25), [0.97, 0.99, 1.0, 1.0];
    Ice: ICE, "ice", Snow, Some(0.5), [0.57, 0.72, 0.98, 1.0];
    PackedIce: PACKED_ICE, "packed_ice", Ice, Some(0.5), [0.55, 0.68, 0.95, 1.0];
    BlueIce: BLUE_ICE, "blue_ice", Ice, Some(2.8), [0.45, 0.62, 0.98, 1.0];
    SpruceLog: SPRUCE_LOG, "spruce_log", Wood, Some(1.2), [0.23, 0.15, 0.08, 1.0];
    BirchLog: BIRCH_LOG, "birch_log", Wood, Some(1.2), [0.85, 0.83, 0.78, 1.0];
    JungleLog: JUNGLE_LOG, "jungle_log", Wood, Some(1.2), [0.33, 0.26, 0.10, 1.0];
    AcaciaLog: ACACIA_LOG, "acacia_log", Wood, Some(1.2), [0.40, 0.38, 0.35, 1.0];
    DarkOakLog: DARK_OAK_LOG, "dark_oak_log", Wood, Some(1.2), [0.24, 0.18, 0.10, 1.0];
    MangroveLog: MANGROVE_LOG, "mangrove_log", Wood, Some(1.2), [0.33, 0.26, 0.17, 1.0];
    CherryLog: CHERRY_LOG, "cherry_log", Wood, Some(1.2), [0.21, 0.13, 0.16, 1.0];
    PaleOakLog: PALE_OAK_LOG, "pale_oak_log", Wood, Some(1.2), [0.38, 0.35, 0.33, 1.0];
    SpruceLeaves: SPRUCE_LEAVES, "spruce_leaves", Leaves, Some(0.25), [0.24, 0.38, 0.24, 1.0];
    BirchLeaves: BIRCH_LEAVES, "birch_leaves", Leaves, Some(0.25), [0.36, 0.50, 0.25, 1.0];
    JungleLeaves: JUNGLE_LEAVES, "jungle_leaves", Leaves, Some(0.25), [0.20, 0.55, 0.10, 1.0];
    AcaciaLeaves: ACACIA_LEAVES, "acacia_leaves", Leaves, Some(0.25), [0.30, 0.50, 0.12, 1.0];
    DarkOakLeaves: DARK_OAK_LEAVES, "dark_oak_leaves", Leaves, Some(0.25), [0.15, 0.40, 0.08, 1.0];
    MangroveLeaves: MANGROVE_LEAVES, "mangrove_leaves", Leaves, Some(0.25), [0.30, 0.50, 0.12, 1.0];
    CherryLeaves: CHERRY_LEAVES, "cherry_leaves", Leaves, Some(0.25), [0.90, 0.65, 0.80, 1.0];
    PaleOakLeaves: PALE_OAK_LEAVES, "pale_oak_leaves", Leaves, Some(0.25), [0.60, 0.62, 0.58, 1.0];
    AzaleaLeaves: AZALEA_LEAVES, "azalea_leaves", Leaves, Some(0.25), [0.35, 0.48, 0.20, 1.0];
    FloweringAzaleaLeaves: FLOWERING_AZALEA_LEAVES, "flowering_azalea_leaves", AzaleaLeaves, Some(0.25), [0.45, 0.48, 0.30, 1.0];
    OakPlanks: OAK_PLANKS, "oak_planks", Wood, Some(1.0), [0.64, 0.51, 0.31, 1.0];
    SprucePlanks: SPRUCE_PLANKS, "spruce_planks", OakPlanks, Some(1.0), [0.45, 0.33, 0.20, 1.0];
    BirchPlanks: BIRCH_PLANKS, "birch_planks", OakPlanks, Some(1.0), [0.76, 0.69, 0.48, 1.0];
    JunglePlanks: JUNGLE_PLANKS, "jungle_planks", OakPlanks, Some(1.0), [0.63, 0.45, 0.31, 1.0];
    AcaciaPlanks: ACACIA_PLANKS, "acacia_planks", OakPlanks, Some(1.0), [0.66, 0.36, 0.20, 1.0];
    DarkOakPlanks: DARK_OAK_PLANKS, "dark_oak_planks", OakPlanks, Some(1.0), [0.26, 0.17, 0.08, 1.0];
    Bricks: BRICKS, "bricks", Stone, Some(2.0), [0.59, 0.38, 0.32, 1.0];
    Prismarine: PRISMARINE, "prismarine", Stone, Some(1.5), [0.39, 0.62, 0.57, 1.0];
    PrismarineBricks: PRISMARINE_BRICKS, "prismarine_bricks", Prismarine, Some(1.5), [0.39, 0.67, 0.59, 1.0];
    DarkPrismarine: DARK_PRISMARINE, "dark_prismarine", Prismarine, Some(1.5), [0.20, 0.36, 0.30, 1.0];
    SeaLantern: SEA_LANTERN, "sea_lantern", Prismarine, Some(0.3), [0.80, 0.87, 0.85, 1.0];
    BrownMushroomBlock: BROWN_MUSHROOM_BLOCK, "brown_mushroom_block", Wood, Some(0.2), [0.58, 0.44, 0.33, 1.0];
    RedMushroomBlock: RED_MUSHROOM_BLOCK, "red_mushroom_block", Wood, Some(0.2), [0.78, 0.18, 0.17, 1.0];
    MushroomStem: MUSHROOM_STEM, "mushroom_stem", Wood, Some(0.2), [0.80, 0.78, 0.72, 1.0];
    Pumpkin: PUMPKIN, "pumpkin", Wood, Some(1.0), [0.78, 0.45, 0.10, 1.0];
    Melon: MELON, "melon", Wood, Some(1.0), [0.45, 0.58, 0.15, 1.0];
    HayBlock: HAY_BLOCK, "hay_block", Wood, Some(0.5), [0.65, 0.55, 0.12, 1.0];
    Bookshelf: BOOKSHELF, "bookshelf", OakPlanks, Some(1.5), [0.50, 0.40, 0.28, 1.0];
    Water: WATER, "water", Stone, None, [0.08, 0.35, 0.72, 0.62];
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
        match key.strip_prefix("minecraft:").unwrap_or(key) {
            "grass" => Some(Self::Grass),
            "wood" => Some(Self::Wood),
            "leaves" => Some(Self::Leaves),
            name => Block::from_registry_key(name).map(|block| Self(block.id)),
        }
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
        if let Some(fallback) = self.legacy_fallback() {
            return fallback;
        }
        let name = self.name();
        for suffix in [
            "_stairs",
            "_slab",
            "_wall",
            "_fence",
            "_fence_gate",
            "_door",
            "_trapdoor",
            "_pane",
        ] {
            if let Some(base) = name.strip_suffix(suffix) {
                for candidate in [
                    base.to_owned(),
                    format!("{base}s"),
                    format!("{base}_planks"),
                ] {
                    if let Some(kind) = Self::from_key(&candidate) {
                        return kind;
                    }
                }
            }
        }
        if name.ends_with("_leaves") {
            Self::Leaves
        } else if name.contains("glass") {
            Self::Ice
        } else if !self.is_solid() {
            Self::Leaves
        } else {
            Self::Stone
        }
    }
}
