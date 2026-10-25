use bevy::prelude::*;
use std::{collections::BTreeSet, sync::Arc};
/// Every block, in one place: `Kind: texture key, fallback, break seconds, vertex color;`.
/// Texture packs look blocks up by key; a pack without one uses the fallback's texture.
macro_rules! voxel_kinds {
    ($($kind:ident: $key:literal, $fallback:ident, $break_duration:expr, $color:expr;)*) => {
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub enum VoxelKind {
            #[default]
            $($kind,)*
        }

        impl VoxelKind {
            pub const ALL: [Self; [$(Self::$kind),*].len()] = [$(Self::$kind),*];

            pub(crate) const fn key(self) -> &'static str {
                match self {
                    $(Self::$kind => $key,)*
                }
            }

            /// Closest block used when a texture pack has no entry for this one.
            pub(crate) const fn fallback(self) -> Self {
                match self {
                    $(Self::$kind => Self::$fallback,)*
                }
            }

            pub(crate) fn from_key(key: &str) -> Option<Self> {
                match key {
                    $($key => Some(Self::$kind),)*
                    _ => None,
                }
            }

            /// Seconds to break by hand; `None` for blocks that cannot be broken.
            pub const fn break_duration(self) -> Option<f32> {
                match self {
                    $(Self::$kind => $break_duration,)*
                }
            }

            pub(crate) const fn color(self) -> [f32; 4] {
                match self {
                    $(Self::$kind => $color,)*
                }
            }
        }
    };
}

voxel_kinds! {
    Air: "air", Stone, None, [0.0, 0.0, 0.0, 0.0];
    Grass: "grass", Stone, Some(0.6), [0.25, 0.65, 0.18, 1.0];
    Dirt: "dirt", Stone, Some(0.5), [0.40, 0.25, 0.12, 1.0];
    Stone: "stone", Stone, Some(1.5), [0.40, 0.40, 0.40, 1.0];
    Sand: "sand", Stone, Some(0.5), [0.76, 0.68, 0.42, 1.0];
    Snow: "snow", Stone, Some(0.3), [0.92, 0.96, 1.0, 1.0];
    Wood: "wood", Stone, Some(1.2), [0.36, 0.20, 0.08, 1.0];
    Leaves: "leaves", Stone, Some(0.25), [0.12, 0.48, 0.10, 1.0];
    Deepslate: "deepslate", Stone, Some(2.0), [0.22, 0.22, 0.25, 1.0];
    Gravel: "gravel", Stone, Some(0.6), [0.48, 0.46, 0.43, 1.0];
    CoalOre: "coal_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    IronOre: "iron_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    CopperOre: "copper_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    GoldOre: "gold_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    RedstoneOre: "redstone_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    DiamondOre: "diamond_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    EmeraldOre: "emerald_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    LapisOre: "lapis_ore", Stone, Some(2.0), [0.48, 0.46, 0.43, 1.0];
    DeepslateCoalOre: "deepslate_coal_ore", CoalOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateIronOre: "deepslate_iron_ore", IronOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateCopperOre: "deepslate_copper_ore", CopperOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateGoldOre: "deepslate_gold_ore", GoldOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateRedstoneOre: "deepslate_redstone_ore", RedstoneOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateDiamondOre: "deepslate_diamond_ore", DiamondOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateEmeraldOre: "deepslate_emerald_ore", EmeraldOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    DeepslateLapisOre: "deepslate_lapis_ore", LapisOre, Some(2.5), [0.25, 0.25, 0.27, 1.0];
    Granite: "granite", Stone, Some(1.5), [0.60, 0.42, 0.35, 1.0];
    Diorite: "diorite", Stone, Some(1.5), [0.75, 0.75, 0.75, 1.0];
    Andesite: "andesite", Stone, Some(1.5), [0.53, 0.53, 0.53, 1.0];
    Tuff: "tuff", Stone, Some(1.5), [0.42, 0.43, 0.38, 1.0];
    Calcite: "calcite", Stone, Some(0.8), [0.87, 0.88, 0.86, 1.0];
    Bedrock: "bedrock", Stone, None, [0.33, 0.33, 0.33, 1.0];
    Cobblestone: "cobblestone", Stone, Some(2.0), [0.48, 0.48, 0.48, 1.0];
    MossyCobblestone: "mossy_cobblestone", Cobblestone, Some(2.0), [0.43, 0.48, 0.38, 1.0];
    CobbledDeepslate: "cobbled_deepslate", Deepslate, Some(3.5), [0.30, 0.30, 0.32, 1.0];
    StoneBricks: "stone_bricks", Stone, Some(1.5), [0.48, 0.48, 0.48, 1.0];
    MossyStoneBricks: "mossy_stone_bricks", StoneBricks, Some(1.5), [0.43, 0.48, 0.38, 1.0];
    CrackedStoneBricks: "cracked_stone_bricks", StoneBricks, Some(1.5), [0.46, 0.46, 0.46, 1.0];
    Obsidian: "obsidian", Stone, Some(10.0), [0.08, 0.06, 0.13, 1.0];
    SmoothBasalt: "smooth_basalt", Stone, Some(1.25), [0.28, 0.28, 0.31, 1.0];
    DripstoneBlock: "dripstone_block", Stone, Some(1.5), [0.53, 0.42, 0.36, 1.0];
    AmethystBlock: "amethyst_block", Stone, Some(1.5), [0.52, 0.38, 0.75, 1.0];
    BuddingAmethyst: "budding_amethyst", AmethystBlock, Some(1.5), [0.52, 0.38, 0.75, 1.0];
    MagmaBlock: "magma_block", Stone, Some(0.5), [0.55, 0.25, 0.10, 1.0];
    Lava: "lava", Stone, None, [0.85, 0.40, 0.08, 1.0];
    CoarseDirt: "coarse_dirt", Dirt, Some(0.5), [0.45, 0.32, 0.22, 1.0];
    RootedDirt: "rooted_dirt", Dirt, Some(0.5), [0.55, 0.40, 0.30, 1.0];
    Podzol: "podzol", Dirt, Some(0.5), [0.36, 0.25, 0.10, 1.0];
    Mycelium: "mycelium", Grass, Some(0.6), [0.44, 0.38, 0.42, 1.0];
    Mud: "mud", Dirt, Some(0.5), [0.24, 0.22, 0.24, 1.0];
    Clay: "clay", Dirt, Some(0.6), [0.62, 0.64, 0.70, 1.0];
    MossBlock: "moss_block", Grass, Some(0.1), [0.35, 0.43, 0.18, 1.0];
    DirtPath: "dirt_path", Dirt, Some(0.65), [0.58, 0.48, 0.25, 1.0];
    Farmland: "farmland", Dirt, Some(0.6), [0.32, 0.20, 0.10, 1.0];
    RedSand: "red_sand", Sand, Some(0.5), [0.75, 0.40, 0.13, 1.0];
    Sandstone: "sandstone", Sand, Some(0.8), [0.85, 0.80, 0.60, 1.0];
    RedSandstone: "red_sandstone", Sandstone, Some(0.8), [0.72, 0.38, 0.12, 1.0];
    Terracotta: "terracotta", Stone, Some(1.25), [0.60, 0.37, 0.27, 1.0];
    WhiteTerracotta: "white_terracotta", Terracotta, Some(1.25), [0.82, 0.70, 0.63, 1.0];
    OrangeTerracotta: "orange_terracotta", Terracotta, Some(1.25), [0.63, 0.33, 0.15, 1.0];
    YellowTerracotta: "yellow_terracotta", Terracotta, Some(1.25), [0.73, 0.52, 0.21, 1.0];
    BrownTerracotta: "brown_terracotta", Terracotta, Some(1.25), [0.30, 0.20, 0.14, 1.0];
    RedTerracotta: "red_terracotta", Terracotta, Some(1.25), [0.56, 0.24, 0.18, 1.0];
    LightGrayTerracotta: "light_gray_terracotta", Terracotta, Some(1.25), [0.53, 0.42, 0.38, 1.0];
    SnowBlock: "snow_block", Snow, Some(0.2), [0.95, 0.98, 1.0, 1.0];
    PowderSnow: "powder_snow", Snow, Some(0.25), [0.97, 0.99, 1.0, 1.0];
    Ice: "ice", Snow, Some(0.5), [0.57, 0.72, 0.98, 1.0];
    PackedIce: "packed_ice", Ice, Some(0.5), [0.55, 0.68, 0.95, 1.0];
    BlueIce: "blue_ice", Ice, Some(2.8), [0.45, 0.62, 0.98, 1.0];
    SpruceLog: "spruce_log", Wood, Some(1.2), [0.23, 0.15, 0.08, 1.0];
    BirchLog: "birch_log", Wood, Some(1.2), [0.85, 0.83, 0.78, 1.0];
    JungleLog: "jungle_log", Wood, Some(1.2), [0.33, 0.26, 0.10, 1.0];
    AcaciaLog: "acacia_log", Wood, Some(1.2), [0.40, 0.38, 0.35, 1.0];
    DarkOakLog: "dark_oak_log", Wood, Some(1.2), [0.24, 0.18, 0.10, 1.0];
    MangroveLog: "mangrove_log", Wood, Some(1.2), [0.33, 0.26, 0.17, 1.0];
    CherryLog: "cherry_log", Wood, Some(1.2), [0.21, 0.13, 0.16, 1.0];
    PaleOakLog: "pale_oak_log", Wood, Some(1.2), [0.38, 0.35, 0.33, 1.0];
    SpruceLeaves: "spruce_leaves", Leaves, Some(0.25), [0.24, 0.38, 0.24, 1.0];
    BirchLeaves: "birch_leaves", Leaves, Some(0.25), [0.36, 0.50, 0.25, 1.0];
    JungleLeaves: "jungle_leaves", Leaves, Some(0.25), [0.20, 0.55, 0.10, 1.0];
    AcaciaLeaves: "acacia_leaves", Leaves, Some(0.25), [0.30, 0.50, 0.12, 1.0];
    DarkOakLeaves: "dark_oak_leaves", Leaves, Some(0.25), [0.15, 0.40, 0.08, 1.0];
    MangroveLeaves: "mangrove_leaves", Leaves, Some(0.25), [0.30, 0.50, 0.12, 1.0];
    CherryLeaves: "cherry_leaves", Leaves, Some(0.25), [0.90, 0.65, 0.80, 1.0];
    PaleOakLeaves: "pale_oak_leaves", Leaves, Some(0.25), [0.60, 0.62, 0.58, 1.0];
    AzaleaLeaves: "azalea_leaves", Leaves, Some(0.25), [0.35, 0.48, 0.20, 1.0];
    FloweringAzaleaLeaves: "flowering_azalea_leaves", AzaleaLeaves, Some(0.25), [0.45, 0.48, 0.30, 1.0];
    OakPlanks: "oak_planks", Wood, Some(1.0), [0.64, 0.51, 0.31, 1.0];
    SprucePlanks: "spruce_planks", OakPlanks, Some(1.0), [0.45, 0.33, 0.20, 1.0];
    BirchPlanks: "birch_planks", OakPlanks, Some(1.0), [0.76, 0.69, 0.48, 1.0];
    JunglePlanks: "jungle_planks", OakPlanks, Some(1.0), [0.63, 0.45, 0.31, 1.0];
    AcaciaPlanks: "acacia_planks", OakPlanks, Some(1.0), [0.66, 0.36, 0.20, 1.0];
    DarkOakPlanks: "dark_oak_planks", OakPlanks, Some(1.0), [0.26, 0.17, 0.08, 1.0];
    Bricks: "bricks", Stone, Some(2.0), [0.59, 0.38, 0.32, 1.0];
    Prismarine: "prismarine", Stone, Some(1.5), [0.39, 0.62, 0.57, 1.0];
    PrismarineBricks: "prismarine_bricks", Prismarine, Some(1.5), [0.39, 0.67, 0.59, 1.0];
    DarkPrismarine: "dark_prismarine", Prismarine, Some(1.5), [0.20, 0.36, 0.30, 1.0];
    SeaLantern: "sea_lantern", Prismarine, Some(0.3), [0.80, 0.87, 0.85, 1.0];
    BrownMushroomBlock: "brown_mushroom_block", Wood, Some(0.2), [0.58, 0.44, 0.33, 1.0];
    RedMushroomBlock: "red_mushroom_block", Wood, Some(0.2), [0.78, 0.18, 0.17, 1.0];
    MushroomStem: "mushroom_stem", Wood, Some(0.2), [0.80, 0.78, 0.72, 1.0];
    Pumpkin: "pumpkin", Wood, Some(1.0), [0.78, 0.45, 0.10, 1.0];
    Melon: "melon", Wood, Some(1.0), [0.45, 0.58, 0.15, 1.0];
    HayBlock: "hay_block", Wood, Some(0.5), [0.65, 0.55, 0.12, 1.0];
    Bookshelf: "bookshelf", OakPlanks, Some(1.5), [0.50, 0.40, 0.28, 1.0];
    Water: "water", Stone, None, [0.08, 0.35, 0.72, 0.62];
}

impl VoxelKind {
    /// Blocks that exist as inventory items, in display order.
    pub const ITEMIZABLE: [Self; Self::ALL.len() - 4] = {
        let mut items = [Self::Air; Self::ALL.len() - 4];
        let (mut from, mut to) = (0, 0);
        while from < Self::ALL.len() {
            if Self::ALL[from].is_itemizable() {
                items[to] = Self::ALL[from];
                to += 1;
            }
            from += 1;
        }
        assert!(to == items.len());
        items
    };

    pub const fn is_itemizable(self) -> bool {
        !matches!(self, Self::Air | Self::Water | Self::Lava | Self::Bedrock)
    }

    pub const fn is_solid(self) -> bool {
        !matches!(self, Self::Air | Self::Water | Self::Lava)
    }

    /// Lava has no fluid simulation yet: it is drawn as an opaque block you sink into.
    pub const fn is_liquid(self) -> bool {
        matches!(self, Self::Water | Self::Lava)
    }

    pub(crate) const fn is_opaque(self) -> bool {
        !matches!(self, Self::Air | Self::Water)
    }
}
/// Blocks per chunk side, fixed by Pumpkin's 16x16 columns.
pub(crate) const CHUNK_SIZE: i32 = 16;
/// Blocks per chunk column: Minecraft Y -64..320, stored from 0.
pub(crate) const CHUNK_HEIGHT: i32 = 384;
/// Height of one render section; a chunk column is meshed one section at a time.
pub(crate) const SECTION_HEIGHT: i32 = 16;
pub(crate) const SECTION_COUNT: usize = (CHUNK_HEIGHT / SECTION_HEIGHT) as usize;
pub(crate) const SECTION_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * SECTION_HEIGHT) as usize;
pub(crate) const CHUNK_VOLUME: usize = SECTION_VOLUME * SECTION_COUNT;
/// Every render section of a column, one bit each.
pub(crate) const ALL_SECTIONS: u32 = ((1u64 << SECTION_COUNT) - 1) as u32;
/// Light above the world and of untouched sky: sky 15, block 0.
pub(crate) const FULL_SKY: u8 = 0xf0;

const _: () = assert!(
    CHUNK_SIZE == 16 && SECTION_HEIGHT == 16,
    "cell indices pack x, z and y as 4-bit fields"
);
const _: () = assert!(
    CHUNK_HEIGHT % SECTION_HEIGHT == 0 && SECTION_COUNT <= 32,
    "sections must tile the column and fit the section mask"
);

/// Index of a local cell in Pumpkin's Y-major layout, `x + 16 * (z + 16 * y)`.
#[inline]
pub(crate) const fn cell_index(local: IVec3) -> Option<usize> {
    if (local.x as u32) < CHUNK_SIZE as u32
        && (local.z as u32) < CHUNK_SIZE as u32
        && (local.y as u32) < CHUNK_HEIGHT as u32
    {
        Some((local.x | local.z << 4 | local.y << 8) as usize)
    } else {
        None
    }
}

/// Inverse of [`cell_index`].
#[inline]
pub(crate) const fn cell_position(index: usize) -> IVec3 {
    IVec3::new(
        (index & 15) as i32,
        (index >> 8) as i32,
        (index >> 4 & 15) as i32,
    )
}

/// Chunk column containing a world cell, and the cell's position inside it.
#[inline]
pub(crate) const fn split_world_position(world: IVec3) -> (IVec2, IVec3) {
    (
        IVec2::new(
            world.x.div_euclid(CHUNK_SIZE),
            world.z.div_euclid(CHUNK_SIZE),
        ),
        IVec3::new(
            world.x.rem_euclid(CHUNK_SIZE),
            world.y,
            world.z.rem_euclid(CHUNK_SIZE),
        ),
    )
}

/// Chunk column containing a continuous world position.
#[inline]
pub fn chunk_at(world: Vec3) -> IVec2 {
    IVec2::new(
        (world.x.floor() as i32).div_euclid(CHUNK_SIZE),
        (world.z.floor() as i32).div_euclid(CHUNK_SIZE),
    )
}

/// One section of a per-cell layer. Most sections hold a single value (open sky,
/// deep stone), stored without its 4096 cells.
#[derive(Clone, Debug)]
pub(crate) enum Section<T> {
    Uniform(T),
    Dense(Arc<[T; SECTION_VOLUME]>),
}

impl<T: Copy + PartialEq> Section<T> {
    fn from_cells(cells: &[T]) -> Self {
        let first = cells[0];
        if cells.iter().all(|&cell| cell == first) {
            Self::Uniform(first)
        } else {
            Self::Dense(Arc::new(cells.try_into().expect("one section of cells")))
        }
    }

    /// Value of a cell; only the low bits of the column index are used.
    #[inline]
    pub(crate) fn get(&self, index: usize) -> T {
        match self {
            Self::Uniform(value) => *value,
            Self::Dense(cells) => cells[index % SECTION_VOLUME],
        }
    }

    #[cfg(test)]
    #[inline]
    pub(crate) fn uniform(&self) -> Option<T> {
        match self {
            Self::Uniform(value) => Some(*value),
            Self::Dense(_) => None,
        }
    }

    /// Copies the cells `start..start + out.len()` of this section.
    #[inline]
    pub(crate) fn copy_cells(&self, start: usize, out: &mut [T]) {
        match self {
            Self::Uniform(value) => out.fill(*value),
            Self::Dense(cells) => out.copy_from_slice(&cells[start..start + out.len()]),
        }
    }

    pub(crate) fn any(&self, predicate: impl Fn(T) -> bool) -> bool {
        match self {
            Self::Uniform(value) => predicate(*value),
            Self::Dense(cells) => cells.iter().any(|&cell| predicate(cell)),
        }
    }

    fn set(&mut self, index: usize, value: T) {
        let index = index % SECTION_VOLUME;
        match self {
            Self::Uniform(fill) => {
                let mut cells = Arc::new([*fill; SECTION_VOLUME]);
                Arc::get_mut(&mut cells).expect("new section")[index] = value;
                *self = Self::Dense(cells);
            }
            Self::Dense(cells) => Arc::make_mut(cells)[index] = value,
        }
    }

    fn same_cells(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Uniform(a), Self::Uniform(b)) => a == b,
            (Self::Dense(a), Self::Dense(b)) => Arc::ptr_eq(a, b) || a == b,
            (Self::Uniform(value), Self::Dense(cells))
            | (Self::Dense(cells), Self::Uniform(value)) => cells.iter().all(|cell| cell == value),
        }
    }

    fn extend_into(&self, out: &mut Vec<T>) {
        match self {
            Self::Uniform(value) => out.extend(std::iter::repeat_n(*value, SECTION_VOLUME)),
            Self::Dense(cells) => out.extend_from_slice(&cells[..]),
        }
    }
}

/// A per-cell layer of a chunk column, compressed by section. Cloning is O(1); an
/// edit copies only its section while a task still reads the previous one.
#[derive(Clone, Debug)]
pub(crate) struct Layer<T>(Arc<[Section<T>; SECTION_COUNT]>);

impl<T: Copy + PartialEq> Layer<T> {
    #[cfg(test)]
    pub(crate) fn filled(value: T) -> Self {
        Self(Arc::new(std::array::from_fn(|_| Section::Uniform(value))))
    }

    /// Compresses a full column in Pumpkin's Y-major layout.
    pub(crate) fn from_cells(cells: &[T]) -> Self {
        assert_eq!(cells.len(), CHUNK_VOLUME, "one full chunk column");
        Self(Arc::new(std::array::from_fn(|section| {
            Section::from_cells(&cells[section * SECTION_VOLUME..][..SECTION_VOLUME])
        })))
    }

    #[inline]
    pub(crate) fn get(&self, index: usize) -> T {
        self.0[index / SECTION_VOLUME].get(index)
    }

    #[inline]
    pub(crate) fn section(&self, section: usize) -> &Section<T> {
        &self.0[section]
    }

    /// Returns whether the cell changed.
    pub(crate) fn set(&mut self, index: usize, value: T) -> bool {
        if self.get(index) == value {
            return false;
        }
        Arc::make_mut(&mut self.0)[index / SECTION_VOLUME].set(index, value);
        true
    }

    /// The full column in Pumpkin's layout, for the lighting worker.
    pub(crate) fn to_cells(&self) -> Arc<[T]> {
        let mut cells = Vec::with_capacity(CHUNK_VOLUME);
        for section in self.0.iter() {
            section.extend_into(&mut cells);
        }
        cells.into()
    }

    /// One bit per section whose cells differ from `other`.
    fn changed_sections(&self, other: &Self) -> u32 {
        if Arc::ptr_eq(&self.0, &other.0) {
            return 0;
        }
        self.0
            .iter()
            .zip(other.0.iter())
            .enumerate()
            .filter(|(_, (old, new))| !old.same_cells(new))
            .fold(0, |mask, (section, _)| mask | (1 << section))
    }
}

/// Chunk data, compressed by section. Cloning it is O(1); edits use copy-on-write
/// while a build reads it.
///
/// Neighbor borders are not copied here: mesh tasks read the neighbors' shared layers.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels {
    cells: Layer<VoxelKind>,
    pub(crate) native_states: Option<Layer<u16>>,
    pub(crate) light: Layer<u8>,
    /// Rendering-only estimates while Pumpkin relights edits. Never seed the worker with these.
    render_light: Option<Layer<u8>>,
    /// Cells edited since generation. Pumpkin replays them whenever a chunk enters its
    /// lighting context, so they are kept for the chunk's whole life.
    pub(crate) native_edits: Arc<BTreeSet<usize>>,
    pub(crate) content_revision: u64,
    /// One bit per render section whose mesh is outdated.
    pub(crate) dirty_sections: u32,
    pub(crate) collider_dirty: bool,
    pub(crate) revision: u64,
    pub(crate) modified: bool,
}
impl ChunkVoxels {
    /// A column of `kind` under open sky, without Minecraft block states.
    #[cfg(test)]
    pub(crate) fn filled(kind: VoxelKind) -> Self {
        Self::from_layers(Layer::filled(kind), Layer::filled(FULL_SKY), None)
    }

    /// A generated column; the slices use Pumpkin's Y-major layout.
    pub(crate) fn generated(cells: &[VoxelKind], states: &[u16], light: &[u8]) -> Self {
        Self::from_layers(
            Layer::from_cells(cells),
            Layer::from_cells(light),
            Some(Layer::from_cells(states)),
        )
    }

    fn from_layers(
        cells: Layer<VoxelKind>,
        light: Layer<u8>,
        native_states: Option<Layer<u16>>,
    ) -> Self {
        Self {
            cells,
            native_states,
            light,
            render_light: None,
            native_edits: default(),
            content_revision: 0,
            dirty_sections: ALL_SECTIONS,
            collider_dirty: true,
            revision: 0,
            modified: false,
        }
    }

    /// Sections whose faces or corner light can read cell `y`.
    pub(crate) const fn sections_around(y: i32) -> u32 {
        const fn section(y: i32) -> u32 {
            if 0 <= y && y < CHUNK_HEIGHT {
                1 << (y / SECTION_HEIGHT)
            } else {
                0
            }
        }
        section(y - 1) | section(y) | section(y + 1)
    }

    /// Requests new meshes for these sections.
    pub(crate) fn mark_sections(&mut self, sections: u32) {
        let sections = sections & ALL_SECTIONS;
        if sections == 0 {
            return;
        }
        self.dirty_sections |= sections;
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn render_light(&self) -> &Layer<u8> {
        self.render_light.as_ref().unwrap_or(&self.light)
    }

    /// Displayed light of a cell, estimates included.
    pub(crate) fn render_light_at(&self, local: IVec3) -> Option<u8> {
        cell_index(local).map(|index| self.render_light().get(index))
    }

    /// The voxel edit already dirties this cell's sections and their border readers.
    pub(crate) fn estimate_light(&mut self, local: IVec3, light: u8) {
        let Some(index) = cell_index(local) else {
            return;
        };
        if self.render_light().get(index) == light {
            return;
        }
        self.render_light
            .get_or_insert_with(|| self.light.clone())
            .set(index, light);
    }

    /// Replaces authoritative light, clears estimates, and returns visually changed sections.
    pub(crate) fn replace_light(&mut self, light: Layer<u8>) -> u32 {
        let changed = self.render_light().changed_sections(&light);
        self.light = light;
        self.render_light = None;
        if changed == 0 {
            return 0;
        }
        // Light is drawn at face corners, which also sample the adjacent sections.
        let spread = changed | (changed << 1) | (changed >> 1);
        self.mark_sections(spread);
        spread & ALL_SECTIONS
    }

    pub fn get(&self, local: IVec3) -> Option<VoxelKind> {
        cell_index(local).map(|index| self.cells.get(index))
    }

    pub(crate) fn cells(&self) -> &Layer<VoxelKind> {
        &self.cells
    }

    /// Changes one cell. Out-of-bounds and identical values return `false`.
    pub fn set(&mut self, local: IVec3, kind: VoxelKind) -> bool {
        let Some(index) = cell_index(local) else {
            return false;
        };
        let old = self.cells.get(index);
        if !self.cells.set(index, kind) {
            return false;
        }
        if let Some(states) = &mut self.native_states {
            states.set(index, super::generation::native_state(kind));
            Arc::make_mut(&mut self.native_edits).insert(index);
        }
        self.content_revision = self.content_revision.wrapping_add(1);
        self.mark_sections(Self::sections_around(local.y));
        if old.is_solid() != kind.is_solid() {
            self.collider_dirty = true;
        }
        self.modified = true;
        true
    }

    /// Solid cells another body can touch. Cells enclosed by six solid neighbors of
    /// this chunk are left out: only a one-cell shell of the terrain is kept. Border
    /// cells stay, since the neighboring chunks are unknown here.
    pub(crate) fn collider_voxels(&self) -> Vec<IVec3> {
        const NEIGHBORS: [IVec3; 6] = [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ];
        let solid =
            |local: IVec3| cell_index(local).is_some_and(|index| self.cells.get(index).is_solid());
        let mut voxels = Vec::new();
        for section in 0..SECTION_COUNT {
            if !self.cells.section(section).any(VoxelKind::is_solid) {
                continue;
            }
            for index in section * SECTION_VOLUME..(section + 1) * SECTION_VOLUME {
                if !self.cells.get(index).is_solid() {
                    continue;
                }
                let local = cell_position(index);
                if !NEIGHBORS.iter().all(|&offset| solid(local + offset)) {
                    voxels.push(local);
                }
            }
        }
        voxels
    }

    #[cfg(test)]
    pub(crate) fn with_light(mut self, light: u8) -> Self {
        self.light = Layer::filled(light);
        self
    }
}
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelChunk {
    pub position: IVec2,
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct VoxelViewer;

#[derive(Event, Clone, Copy, Debug)]
pub struct SetVoxel {
    pub world_position: IVec3,
    pub kind: VoxelKind,
}

impl SetVoxel {
    pub const fn new(world_position: IVec3, kind: VoxelKind) -> Self {
        Self {
            world_position,
            kind,
        }
    }
}

#[cfg(test)]
mod tests {
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
            (2..SECTION_COUNT)
                .all(|section| layer.section(section).uniform() == Some(VoxelKind::Air))
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
}
