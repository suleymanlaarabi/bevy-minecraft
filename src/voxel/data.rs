use bevy::prelude::*;
use std::sync::Arc;
type VoxelBuffer = Vec<VoxelKind>;
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

    pub const fn is_leaves(self) -> bool {
        matches!(
            self,
            Self::AcaciaLeaves
                | Self::AzaleaLeaves
                | Self::BirchLeaves
                | Self::CherryLeaves
                | Self::DarkOakLeaves
                | Self::FloweringAzaleaLeaves
                | Self::JungleLeaves
                | Self::Leaves
                | Self::MangroveLeaves
                | Self::PaleOakLeaves
                | Self::SpruceLeaves
        )
    }

    pub const fn is_solid(self) -> bool {
        !matches!(self, Self::Air | Self::Water | Self::Lava) && !self.is_leaves()
    }

    /// Lava has no fluid simulation yet: it is drawn as an opaque block you sink into.
    pub const fn is_liquid(self) -> bool {
        matches!(self, Self::Water | Self::Lava)
    }

    pub(crate) const fn is_opaque(self) -> bool {
        !matches!(self, Self::Air | Self::Water)
    }
}
/// Height of one render section; a chunk column is meshed one section at a time.
pub(crate) const SECTION_HEIGHT: i32 = 16;

/// Dense chunk data. Cloning it is O(1); edits use copy-on-write while a build reads it.
///
/// Neighbor borders are not copied here: mesh tasks read the neighbors' shared arrays.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels {
    pub(crate) size: i32,
    pub(crate) height: i32,
    cells: Arc<[VoxelKind]>,
    pub(crate) native_states: Option<Arc<[u16]>>,
    pub(crate) light: Arc<[u8]>,
    pub(crate) native_edits: Arc<std::collections::BTreeSet<usize>>,
    pub(crate) content_revision: u64,
    /// One bit per render section whose mesh is outdated.
    pub(crate) dirty_sections: u32,
    pub(crate) collider_dirty: bool,
    pub(crate) revision: u64,
    pub(crate) modified: bool,
}
impl ChunkVoxels {
    pub(crate) fn generated(size: i32, height: i32, cells: VoxelBuffer) -> Self {
        assert_eq!(cells.len(), (size * size * height) as usize);
        let mut voxels = Self {
            size,
            height,
            cells: cells.into(),
            native_states: None,
            light: vec![0xf0; (size * size * height) as usize].into(),
            native_edits: default(),
            content_revision: 0,
            dirty_sections: 0,
            collider_dirty: true,
            revision: 0,
            modified: false,
        };
        assert!(
            voxels.section_count() <= 32,
            "chunk too tall for the section mask"
        );
        voxels.dirty_sections = voxels.all_sections();
        voxels
    }

    pub(crate) fn with_lighting(mut self, states: Arc<[u16]>, light: Arc<[u8]>) -> Self {
        self.native_states = Some(states);
        self.light = light;
        self
    }

    pub(crate) fn section_count(&self) -> usize {
        (self.height + SECTION_HEIGHT - 1).div_euclid(SECTION_HEIGHT) as usize
    }

    pub(crate) fn all_sections(&self) -> u32 {
        (((1u64) << self.section_count()) - 1) as u32
    }

    /// Sections whose faces or corner light can read cell `y`.
    pub(crate) fn sections_around(&self, y: i32) -> u32 {
        let section = |y: i32| {
            (0..self.height)
                .contains(&y)
                .then(|| 1u32 << y.div_euclid(SECTION_HEIGHT))
                .unwrap_or(0)
        };
        section(y - 1) | section(y) | section(y + 1)
    }

    /// Requests new meshes for these sections.
    pub(crate) fn mark_sections(&mut self, sections: u32) {
        let sections = sections & self.all_sections();
        if sections == 0 {
            return;
        }
        self.dirty_sections |= sections;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Replaces the light and returns the sections whose light changed.
    pub(crate) fn replace_light(&mut self, light: Arc<[u8]>) -> u32 {
        if self.light == light || light.len() != self.light.len() {
            return 0;
        }
        let section_cells = (self.size * self.size * SECTION_HEIGHT) as usize;
        let changed = self
            .light
            .chunks(section_cells)
            .zip(light.chunks(section_cells))
            .enumerate()
            .filter(|(_, (old, new))| old != new)
            .fold(0, |mask, (section, _)| mask | (1 << section));
        self.light = light;
        // Light is drawn at face corners, which also sample the adjacent sections.
        let spread = changed | (changed << 1) | (changed >> 1);
        self.mark_sections(spread);
        spread & self.all_sections()
    }

    pub fn get(&self, local: IVec3) -> Option<VoxelKind> {
        self.index(local).map(|index| self.cells[index])
    }

    pub(crate) fn cells(&self) -> &Arc<[VoxelKind]> {
        &self.cells
    }

    /// Changes one cell. Out-of-bounds and identical values return `false`.
    pub fn set(&mut self, local: IVec3, kind: VoxelKind) -> bool {
        let Some(index) = self.index(local) else {
            return false;
        };
        let old = self.cells[index];
        if old == kind {
            return false;
        }
        Arc::make_mut(&mut self.cells)[index] = kind;
        if let Some(states) = &mut self.native_states {
            Arc::make_mut(states)[index] = super::generation::native_state(kind);
            Arc::make_mut(&mut self.native_edits).insert(index);
        }
        self.content_revision = self.content_revision.wrapping_add(1);
        self.mark_sections(self.sections_around(local.y));
        if old.is_solid() != kind.is_solid() {
            self.collider_dirty = true;
        }
        self.modified = true;
        true
    }

    pub(crate) fn solid_positions(&self) -> Vec<IVec3> {
        let size = self.size as usize;
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.is_solid())
            .map(|(index, _)| {
                IVec3::new(
                    (index % size) as i32,
                    (index / (size * size)) as i32,
                    (index / size % size) as i32,
                )
            })
            .collect()
    }

    pub(crate) fn index(&self, local: IVec3) -> Option<usize> {
        if local.x < 0
            || local.x >= self.size
            || local.y < 0
            || local.y >= self.height
            || local.z < 0
            || local.z >= self.size
        {
            return None;
        }
        Some(
            local.x as usize
                + self.size as usize * (local.z as usize + self.size as usize * local.y as usize),
        )
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
        let mut chunk = ChunkVoxels::generated(1, 48, vec![VoxelKind::Air; 48]);
        chunk.dirty_sections = 0;
        let mut light = vec![0xf0; 48];
        light[40] = 0x5c;
        assert_eq!(chunk.replace_light(light.clone().into()), 0b110);
        assert_eq!(chunk.dirty_sections, 0b110);
        assert_eq!(chunk.content_revision, 0);
        assert!(!chunk.modified);
        assert_eq!(chunk.revision, 1);
        assert_eq!(chunk.replace_light(light.into()), 0);
        assert_eq!(chunk.revision, 1);
    }

    #[test]
    fn edits_advance_revision_and_only_dirty_required_outputs() {
        let mut chunk = ChunkVoxels::generated(2, 32, vec![VoxelKind::Air; 128]);
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
}
