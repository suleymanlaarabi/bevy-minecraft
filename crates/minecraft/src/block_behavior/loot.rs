//! Small, precompiled loot policies shared by player mining and support removal.
use crate::{
    inventory::{ItemKind, ItemStack},
    voxel::{VoxelKind, VoxelState},
};
use bevy::prelude::*;
use pumpkin_data::{BlockId, PropertyKey as K, tag::Block as Tag};
use std::sync::OnceLock;

#[derive(Clone, Copy)]
enum LootRule {
    None,
    Fixed(ItemKind),
    Sheared(ItemKind),
    Leaves {
        block: ItemKind,
        sapling: Option<ItemKind>,
    },
    Gravel(ItemKind),
    Crop {
        seed: ItemKind,
        produce: ItemKind,
        maximum: pumpkin_data::Property,
    },
}
fn catalog() -> &'static [LootRule] {
    static RULES: OnceLock<Box<[LootRule]>> = OnceLock::new();
    RULES.get_or_init(|| {
        VoxelKind::ALL
            .iter()
            .map(|&kind| {
                let name = kind.name();
                let item = ItemKind::from_voxel(kind);
                if kind.tagged(Tag::MINECRAFT_LEAVES) {
                    let sapling_name = match name {
                        "mangrove_leaves" => "mangrove_propagule".to_owned(),
                        "azalea_leaves" => "azalea".to_owned(),
                        "flowering_azalea_leaves" => "flowering_azalea".to_owned(),
                        _ => name.replace("_leaves", "_sapling"),
                    };
                    return item.map_or(LootRule::None, |block| LootRule::Leaves {
                        block,
                        sapling: ItemKind::from_name(&sapling_name),
                    });
                }
                if kind == VoxelKind::Gravel {
                    return LootRule::Gravel(ItemKind::from_name("flint").expect("Flint item"));
                }
                let crop = match name {
                    "wheat" => Some(("wheat_seeds", "wheat")),
                    "beetroots" => Some(("beetroot_seeds", "beetroot")),
                    "carrots" => Some(("carrot", "carrot")),
                    "potatoes" => Some(("potato", "potato")),
                    "nether_wart" => Some(("nether_wart", "nether_wart")),
                    "cocoa" => Some(("cocoa_beans", "cocoa_beans")),
                    "torchflower_crop" => Some(("torchflower_seeds", "torchflower")),
                    "pitcher_crop" => Some(("pitcher_pod", "pitcher_plant")),
                    _ => None,
                };
                if let Some((seed, produce)) = crop {
                    let maximum = kind
                        .block()
                        .states
                        .iter()
                        .filter_map(|state| {
                            VoxelState::from_id(state.id.as_u16())
                                .and_then(|state| state.property(K::Age))
                        })
                        .max_by_key(|age| match age {
                            pumpkin_data::Property::Age(value) => *value,
                            _ => 0,
                        });
                    if let (Some(seed), Some(produce), Some(maximum)) = (
                        ItemKind::from_name(seed),
                        ItemKind::from_name(produce),
                        maximum,
                    ) {
                        return LootRule::Crop {
                            seed,
                            produce,
                            maximum,
                        };
                    }
                    return LootRule::None;
                }
                if matches!(
                    kind.native_id().unwrap(),
                    BlockId::COBWEB
                        | BlockId::VINE
                        | BlockId::GLOW_LICHEN
                        | BlockId::SEAGRASS
                        | BlockId::TALL_SEAGRASS
                        | BlockId::FERN
                        | BlockId::LARGE_FERN
                        | BlockId::DEAD_BUSH
                        | BlockId::SHORT_GRASS
                        | BlockId::TALL_GRASS
                        | BlockId::HANGING_ROOTS
                        | BlockId::NETHER_SPROUTS
                ) {
                    return item.map_or(LootRule::None, LootRule::Sheared);
                }
                let ore_name = name.strip_prefix("deepslate_").unwrap_or(name);
                let drop = match ore_name {
                    "stone" => Some("cobblestone"),
                    "deepslate" => Some("cobbled_deepslate"),
                    "grass_block" | "farmland" | "dirt_path" | "podzol" | "mycelium" => {
                        Some("dirt")
                    }
                    "coal_ore" => Some("coal"),
                    "iron_ore" => Some("raw_iron"),
                    "copper_ore" => Some("raw_copper"),
                    "gold_ore" => Some("raw_gold"),
                    "diamond_ore" => Some("diamond"),
                    "emerald_ore" => Some("emerald"),
                    "redstone_ore" => Some("redstone"),
                    "lapis_ore" => Some("lapis_lazuli"),
                    "nether_quartz_ore" => Some("quartz"),
                    "nether_gold_ore" => Some("gold_nugget"),
                    "amethyst_cluster" => Some("amethyst_shard"),
                    _ => None,
                };
                if let Some(drop) = drop {
                    return ItemKind::from_name(drop).map_or(LootRule::None, LootRule::Fixed);
                }
                if matches!(
                    kind.native_id().unwrap(),
                    BlockId::BUDDING_AMETHYST
                        | BlockId::GLASS
                        | BlockId::GLASS_PANE
                        | BlockId::ICE
                        | BlockId::PACKED_ICE
                        | BlockId::BLUE_ICE
                ) || name.ends_with("_stained_glass")
                    || name.ends_with("_stained_glass_pane")
                    || kind.block().loot_table.is_none()
                {
                    return LootRule::None;
                }
                item.map_or(LootRule::None, LootRule::Fixed)
            })
            .collect()
    })
}

/// The state belongs to the checked edit; paired blocks only request one loot evaluation.
#[derive(Clone, Copy)]
pub(crate) struct Harvest {
    pub position: IVec3,
    pub state: VoxelState,
    pub offset: Vec3,
    pub velocity: Vec3,
}
#[derive(Resource)]
pub(crate) struct LootRandom(u64);
impl Default for LootRandom {
    fn default() -> Self {
        Self(0x9e3779b97f4a7c15)
    }
}
impl LootRandom {
    fn chance(&mut self, percent: u64) -> bool {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        (value ^ (value >> 31)) % 100 < percent
    }
}
pub(crate) fn drops(
    state: VoxelState,
    shears: bool,
    random: &mut LootRandom,
) -> [Option<ItemStack>; 2] {
    let (first, second) = match catalog()
        .get(state.kind().index())
        .copied()
        .unwrap_or(LootRule::None)
    {
        LootRule::None => (None, None),
        LootRule::Sheared(item) => (shears.then(|| ItemStack::new(item, 1)), None),
        LootRule::Fixed(item) => (
            Some(ItemStack::new(
                item,
                if state.property(K::Type)
                    == Some(pumpkin_data::Property::SlabType(
                        pumpkin_data::block_properties::SlabType::Double,
                    ))
                {
                    2
                } else {
                    1
                },
            )),
            None,
        ),
        LootRule::Leaves { block, sapling } => (
            if shears {
                Some(ItemStack::new(block, 1))
            } else if random.chance(5) {
                sapling.map(|item| ItemStack::new(item, 1))
            } else {
                None
            },
            None,
        ),
        LootRule::Gravel(flint) => (
            Some(ItemStack::new(
                if random.chance(10) {
                    flint
                } else {
                    ItemKind::for_block(VoxelKind::Gravel)
                },
                1,
            )),
            None,
        ),
        LootRule::Crop {
            seed,
            produce,
            maximum,
        } => {
            if state.property(K::Age) != Some(maximum) {
                (Some(ItemStack::new(seed, 1)), None)
            } else if seed == produce {
                (Some(ItemStack::new(produce, 2)), None)
            } else {
                (
                    Some(ItemStack::new(produce, 1)),
                    Some(ItemStack::new(seed, 1)),
                )
            }
        }
    };
    [first, second]
}
pub(crate) fn initialize() {
    let _ = catalog();
}
