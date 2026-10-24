use bevy::prelude::*;
use std::sync::Arc;
pub(crate) const MESH_CHANGED: u8 = 1;
pub(crate) const COLLIDER_CHANGED: u8 = 2;
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
/// Dense chunk data. Cloning it is O(1); edits use copy-on-write while a build reads it.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels {
    pub(crate) size: i32,
    pub(crate) height: i32,
    cells: Arc<[VoxelKind]>,
    halo: Arc<[VoxelKind]>,
    pub(crate) changes: u8,
    pub(crate) revision: u64,
    pub(crate) modified: bool,
}
impl ChunkVoxels {
    pub(crate) fn generated(size: i32, height: i32, cells: VoxelBuffer, halo: VoxelBuffer) -> Self {
        Self {
            size,
            height,
            cells: cells.into(),
            halo: halo.into(),
            changes: MESH_CHANGED | COLLIDER_CHANGED,
            revision: 0,
            modified: false,
        }
    }

    pub fn get(&self, local: IVec3) -> Option<VoxelKind> {
        self.index(local).map(|index| self.cells[index])
    }

    pub(crate) fn sample(&self, local: IVec3) -> VoxelKind {
        self.get(local)
            .or_else(|| self.halo_kind(local))
            .unwrap_or(VoxelKind::Air)
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
        self.revision = self.revision.wrapping_add(1);
        self.changes |= MESH_CHANGED;
        if old.is_solid() != kind.is_solid() {
            self.changes |= COLLIDER_CHANGED;
        }
        self.modified = true;
        true
    }

    /// Updates a non-owning neighbor sample used only by the mesher.
    pub(crate) fn set_halo(&mut self, local: IVec3, kind: VoxelKind) -> bool {
        let Some(index) = self.halo_index(local) else {
            return false;
        };
        if self.halo[index] == kind {
            return false;
        }
        Arc::make_mut(&mut self.halo)[index] = kind;
        self.revision = self.revision.wrapping_add(1);
        self.changes |= MESH_CHANGED;
        true
    }

    /// Copies one complete border from an adjacent chunk into this chunk's halo.
    pub(crate) fn sync_halo(&mut self, direction: IVec2, neighbor: &Self) -> bool {
        let (halo, cell, step) = match direction {
            IVec2::NEG_X => (IVec3::NEG_X, IVec3::new(self.size - 1, 0, 0), IVec3::Z),
            IVec2::X => (IVec3::new(self.size, 0, 0), IVec3::ZERO, IVec3::Z),
            IVec2::NEG_Y => (IVec3::NEG_Z, IVec3::new(0, 0, self.size - 1), IVec3::X),
            IVec2::Y => (IVec3::new(0, 0, self.size), IVec3::ZERO, IVec3::X),
            _ => unreachable!("chunk neighbors are cardinal"),
        };
        let mut changed = false;
        for y in 0..self.height {
            for offset in 0..self.size {
                let offset = IVec3::Y * y + step * offset;
                changed |= self.set_halo(halo + offset, neighbor.get(cell + offset).unwrap());
            }
        }
        changed
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

    fn index(&self, local: IVec3) -> Option<usize> {
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

    fn halo_index(&self, local: IVec3) -> Option<usize> {
        if !(0..self.height).contains(&local.y) {
            return None;
        }
        let size = self.size as usize;
        let (side, offset) = match (local.x, local.z) {
            (-1, z) if (0..self.size).contains(&z) => (0, z),
            (x, z) if x == self.size && (0..self.size).contains(&z) => (1, z),
            (x, -1) if (0..self.size).contains(&x) => (2, x),
            (x, z) if z == self.size && (0..self.size).contains(&x) => (3, x),
            _ => return None,
        };
        Some(offset as usize + size * (side + 4 * local.y as usize))
    }

    fn halo_kind(&self, local: IVec3) -> Option<VoxelKind> {
        self.halo_index(local).map(|index| self.halo[index])
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
    fn edits_advance_revision_and_only_dirty_required_outputs() {
        let mut chunk =
            ChunkVoxels::generated(2, 2, vec![VoxelKind::Air; 8], vec![VoxelKind::Air; 16]);
        chunk.changes = 0;

        assert!(chunk.set(IVec3::ZERO, VoxelKind::Stone));
        assert_eq!(chunk.revision, 1);
        assert_eq!(chunk.changes, MESH_CHANGED | COLLIDER_CHANGED);
        assert!(chunk.modified);

        chunk.changes = 0;
        assert!(!chunk.set(IVec3::ZERO, VoxelKind::Stone));
        assert_eq!(chunk.revision, 1);
        assert_eq!(chunk.changes, 0);

        assert!(chunk.set_halo(IVec3::new(-1, 0, 0), VoxelKind::Stone));
        assert_eq!(chunk.revision, 2);
        assert_eq!(chunk.changes, MESH_CHANGED);
    }
}
