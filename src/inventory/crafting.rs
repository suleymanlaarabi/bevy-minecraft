//! Compile vanilla recipes into small, allocation-free 2x2 matchers once.
use super::{ItemKind, ItemStack};
use pumpkin_data::{
    item::Item,
    recipes::{CraftingRecipeTypes, RECIPES_CRAFTING, RecipeIngredientTypes},
};
use std::{collections::HashMap, sync::OnceLock};

pub(crate) struct Recipe {
    pub output: ItemStack,
    ingredients: Vec<Vec<u16>>,
    shapeless: bool,
}

fn ingredient(
    value: &RecipeIngredientTypes,
    tags: &mut HashMap<&'static str, Vec<u16>>,
) -> Vec<u16> {
    let mut ids = match value {
        RecipeIngredientTypes::Simple(name) => Item::from_registry_key(name)
            .map(|item| vec![item.id])
            .unwrap_or_default(),
        RecipeIngredientTypes::OneOf(names) => names
            .iter()
            .filter_map(|name| Item::from_registry_key(name))
            .map(|item| item.id)
            .collect(),
        RecipeIngredientTypes::Tagged(tag) => tags
            .entry(tag)
            .or_insert_with(|| {
                (0..super::model::ITEM_ID_LIMIT)
                    .filter_map(Item::from_id)
                    .filter(|item| value.match_item(item))
                    .map(|item| item.id)
                    .collect()
            })
            .clone(),
    };
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn recipes() -> &'static [Recipe] {
    static RECIPES: OnceLock<Vec<Recipe>> = OnceLock::new();
    RECIPES.get_or_init(|| {
        let mut recipes = Vec::new();
        let mut tags = HashMap::new();
        for recipe in RECIPES_CRAFTING {
            match recipe {
                CraftingRecipeTypes::CraftingShaped {
                    key,
                    pattern,
                    result,
                    ..
                } => {
                    let Some(item) = ItemKind::from_name(result.id) else {
                        continue;
                    };
                    let height = pattern.len();
                    let width = pattern.first().map_or(0, |row| row.len());
                    if height == 0 || height > 2 || width == 0 || width > 2 {
                        continue;
                    }
                    let output = ItemStack::new(item, result.count);
                    for oy in 0..=2 - height {
                        for ox in 0..=2 - width {
                            // A one-wide pattern is its own mirror.
                            for mirrored in [false, true].into_iter().take(width) {
                                let mut ingredients = vec![Vec::new(); 4];
                                for (y, row) in pattern.iter().enumerate() {
                                    for (x, symbol) in row.chars().enumerate() {
                                        let x = if mirrored { width - 1 - x } else { x };
                                        if let Some((_, value)) =
                                            key.iter().find(|(c, _)| *c == symbol)
                                        {
                                            ingredients[(y + oy) * 2 + x + ox] =
                                                ingredient(value, &mut tags);
                                        }
                                    }
                                }
                                recipes.push(Recipe {
                                    output,
                                    ingredients,
                                    shapeless: false,
                                });
                            }
                        }
                    }
                }
                CraftingRecipeTypes::CraftingShapeless {
                    ingredients,
                    result,
                    ..
                } if ingredients.len() <= 4 => {
                    let Some(item) = ItemKind::from_name(result.id) else {
                        continue;
                    };
                    recipes.push(Recipe {
                        output: ItemStack::new(item, result.count),
                        ingredients: ingredients
                            .iter()
                            .map(|value| ingredient(value, &mut tags))
                            .collect(),
                        shapeless: true,
                    });
                }
                _ => {}
            }
        }
        recipes
    })
}

pub(crate) fn get(index: usize) -> &'static Recipe {
    &recipes()[index]
}

/// Builds the recipe and stack-size tables up front instead of on the first click.
pub(crate) fn initialize() {
    let _ = recipes();
    let _ = ItemKind::Block(crate::voxel::VoxelKind::Dirt).max_stack_size();
}

// Backtracking is bounded by four slots and handles overlapping ingredient tags.
fn match_shapeless(ingredients: &[Vec<u16>], ids: &[Option<u16>; 4], used: u8) -> bool {
    let Some((ingredient, rest)) = ingredients.split_first() else {
        return true;
    };
    ids.iter().enumerate().any(|(index, id)| {
        used & (1 << index) == 0
            && id.is_some_and(|id| ingredient.binary_search(&id).is_ok())
            && match_shapeless(rest, ids, used | (1 << index))
    })
}

/// Index of the recipe matching the 2x2 grid.
pub(crate) fn find(slots: &[Option<ItemStack>]) -> Option<usize> {
    let ids: [Option<u16>; 4] = std::array::from_fn(|i| slots[i].map(|s| s.item().registry().id));
    let occupied = ids.iter().flatten().count();
    if occupied == 0 {
        return None;
    }
    recipes().iter().position(|recipe| {
        if recipe.shapeless {
            occupied == recipe.ingredients.len() && match_shapeless(&recipe.ingredients, &ids, 0)
        } else {
            recipe
                .ingredients
                .iter()
                .zip(ids)
                .all(|(ingredient, id)| match id {
                    Some(id) => ingredient.binary_search(&id).is_ok(),
                    None => ingredient.is_empty(),
                })
        }
    })
}
