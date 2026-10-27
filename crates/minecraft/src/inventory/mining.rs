//! Immutable tool/block tables; registry components and tags are resolved before play.
use super::{ItemKind, ItemStack};
use crate::{
    character::CharacterState,
    voxel::{VoxelKind, VoxelState},
};
use pumpkin_data::{
    data_component_impl::{IDSet, ToolImpl},
    tag::Taggable,
};
use std::sync::OnceLock;

#[derive(Clone, Copy)]
struct ToolBlock {
    speed: f32,
    correct: bool,
}
struct ToolProfile {
    blocks: Box<[ToolBlock]>,
    damage: u32,
    shears: bool,
}
struct Catalog {
    items: Box<[u8]>,
    profiles: Vec<ToolProfile>,
}
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let bare = VoxelKind::all()
            .collect::<Vec<_>>()
            .iter()
            .map(|_| ToolBlock {
                speed: 1.0,
                correct: false,
            })
            .collect();
        let mut result = Catalog {
            items: vec![0; super::model::ITEM_ID_LIMIT as usize].into_boxed_slice(),
            profiles: vec![ToolProfile {
                blocks: bare,
                damage: 0,
                shears: false,
            }],
        };
        let mut sources: Vec<&ToolImpl> = Vec::new();
        for id in 0..super::model::ITEM_ID_LIMIT {
            let Some(item) = pumpkin_data::item::Item::from_id(id) else {
                continue;
            };
            let name = item.registry_key;
            if name != "shears"
                && !["_pickaxe", "_axe", "_shovel", "_hoe"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
            {
                continue;
            }
            let Some(tool) = ItemKind(id).component::<ToolImpl>() else {
                continue;
            };
            let index = if let Some(index) = sources.iter().position(|source| **source == *tool) {
                index + 1
            } else {
                let blocks = VoxelKind::ALL
                    .iter()
                    .map(|kind| {
                        let matches = |rule: &&pumpkin_data::data_component_impl::ToolRule| {
                            match &rule.blocks {
                                IDSet::Tag(tag) => {
                                    kind.is_native()
                                        && kind
                                            .block()
                                            .is_tagged_with(tag)
                                            .expect("Valid tool block tag")
                                }
                                IDSet::IDs(blocks) => blocks
                                    .iter()
                                    .any(|block| u32::from(block.id.as_u16()) == kind.0),
                            }
                        };
                        let speed = tool
                            .rules
                            .iter()
                            .filter(matches)
                            .find_map(|rule| rule.speed)
                            .unwrap_or(tool.default_mining_speed);
                        let correct = tool
                            .rules
                            .iter()
                            .filter(matches)
                            .find_map(|rule| rule.correct_for_drops)
                            .unwrap_or(false);
                        ToolBlock { speed, correct }
                    })
                    .collect();
                result.profiles.push(ToolProfile {
                    blocks,
                    damage: tool.damage_per_block,
                    shears: name == "shears",
                });
                sources.push(tool);
                result.profiles.len() - 1
            };
            result.items[id as usize] = index
                .try_into()
                .expect("At most 256 distinct gathering tool profiles");
        }
        result
    })
}
#[derive(Clone, Copy)]
pub(crate) struct MiningOutcome {
    pub duration: f32,
    pub can_harvest: bool,
    pub damage: u32,
    pub shears: bool,
}
pub(crate) fn outcome(
    state: VoxelState,
    tool: Option<ItemStack>,
    player: &CharacterState,
) -> Option<MiningOutcome> {
    let kind = state.kind();
    let hardness = state.definition().hardness;
    if hardness < 0.0 || state.definition().is_air() || kind.is_liquid() {
        return None;
    }
    let catalog = catalog();
    let profile_id = tool.map_or(0, |stack| catalog.items[stack.item().id() as usize]);
    let profile = &catalog.profiles[usize::from(profile_id)];
    let block = profile
        .blocks
        .get(kind.index())
        .copied()
        .unwrap_or(ToolBlock {
            speed: 1.0,
            correct: false,
        });
    let mut duration = hardness / block.speed.max(f32::MIN_POSITIVE);
    let can_harvest = !state.definition().tool_required() || block.correct;
    if !can_harvest {
        duration *= 3.0;
    }
    if player.eyes_in_swimmable_fluid {
        duration *= 5.0;
    }
    if !player.on_ground {
        duration *= 5.0;
    }
    Some(MiningOutcome {
        duration,
        can_harvest,
        damage: if hardness > 0.0 { profile.damage } else { 0 },
        shears: profile.shears,
    })
}
pub(crate) fn initialize() {
    let _ = catalog();
}
