//! Shared, indexed crafting over fixed grids and transaction buffers.
use super::{INVENTORY_SIZE, ItemKind, ItemStack, PlayerInventory, slots};
use bevy::prelude::*;
use pumpkin_data::{
    item::Item,
    recipes::{CraftingRecipeTypes, RECIPES_CRAFTING, RecipeIngredientTypes},
};
use std::{collections::HashMap, sync::OnceLock};

const EMPTY: u16 = u16::MAX;
const WORDS: usize = super::model::ITEM_ID_LIMIT as usize / 64;
type Ingredient = [u64; WORDS];

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CraftingGrid {
    pub slots: [Option<ItemStack>; 9],
    pub side: usize,
}
impl Default for CraftingGrid {
    fn default() -> Self {
        Self {
            slots: [None; 9],
            side: 2,
        }
    }
}
impl CraftingGrid {
    pub fn active_len(&self) -> usize {
        self.side * self.side
    }
    pub fn recipe(&self) -> Option<RecipeMatch> {
        find(self)
    }
}
#[derive(Component, Default, PartialEq, Eq)]
pub(crate) struct CraftingResult(pub Option<ItemStack>);

#[derive(Clone, Copy)]
pub(crate) struct RecipeMatch {
    recipe: usize,
    consumed: u16,
}
impl RecipeMatch {
    pub fn output(self) -> ItemStack {
        catalog().recipes[self.recipe].output
    }
}
struct Recipe {
    output: ItemStack,
    ingredients: [u16; 9],
    width: usize,
    occupied: usize,
}
struct Catalog {
    recipes: Vec<Recipe>,
    ingredients: Vec<Ingredient>,
    shaped: [Vec<usize>; 16],
    shapeless: [Vec<usize>; 10],
}

fn compile_ingredient(
    value: &RecipeIngredientTypes,
    intern: &mut HashMap<Ingredient, u16>,
    ingredients: &mut Vec<Ingredient>,
    tags: &mut HashMap<&'static str, u16>,
) -> u16 {
    if let RecipeIngredientTypes::Tagged(tag) = value
        && let Some(&index) = tags.get(tag)
    {
        return index;
    }
    let mut bits = [0; WORDS];
    let mut add = |item: &'static Item| {
        bits[item.id as usize / 64] |= 1 << (item.id % 64);
    };
    match value {
        RecipeIngredientTypes::Simple(name) => {
            add(Item::from_registry_key(name).expect("Recipe ingredient must exist"))
        }
        RecipeIngredientTypes::OneOf(names) => {
            for name in *names {
                add(Item::from_registry_key(name).expect("Recipe ingredient must exist"));
            }
        }
        RecipeIngredientTypes::Tagged(_) => {
            for item in (0..super::model::ITEM_ID_LIMIT).filter_map(Item::from_id) {
                if value.match_item(item) {
                    add(item);
                }
            }
        }
    }
    let index = *intern.entry(bits).or_insert_with(|| {
        let index = ingredients.len() as u16;
        ingredients.push(bits);
        index
    });
    if let RecipeIngredientTypes::Tagged(tag) = value {
        tags.insert(tag, index);
    }
    index
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut result = Catalog {
            recipes: Vec::new(),
            ingredients: Vec::new(),
            shaped: std::array::from_fn(|_| Vec::new()),
            shapeless: std::array::from_fn(|_| Vec::new()),
        };
        let mut intern = HashMap::new();
        let mut tags = HashMap::new();
        for source in RECIPES_CRAFTING {
            let (output, ingredients, width, height) = match source {
                CraftingRecipeTypes::CraftingShaped {
                    key,
                    pattern,
                    result: output,
                    ..
                } => {
                    let item =
                        ItemKind::from_name(output.id).expect("Crafting recipe result must exist");
                    let mut min = [3usize; 2];
                    let mut max = [0usize; 2];
                    for (y, row) in pattern.iter().enumerate() {
                        for (x, symbol) in row.chars().enumerate() {
                            if symbol != ' ' {
                                min[0] = min[0].min(x);
                                min[1] = min[1].min(y);
                                max[0] = max[0].max(x + 1);
                                max[1] = max[1].max(y + 1);
                            }
                        }
                    }
                    if max[0] <= min[0] || max[1] <= min[1] || max[0] > 3 || max[1] > 3 {
                        continue;
                    }
                    let width = max[0] - min[0];
                    let height = max[1] - min[1];
                    let mut ingredients = [EMPTY; 9];
                    for (y, row) in pattern.iter().enumerate().take(max[1]).skip(min[1]) {
                        for (x, symbol) in row.chars().enumerate().take(max[0]).skip(min[0]) {
                            if symbol != ' ' {
                                let value = &key
                                    .iter()
                                    .find(|(c, _)| *c == symbol)
                                    .expect("Valid recipe key")
                                    .1;
                                ingredients[(y - min[1]) * width + x - min[0]] = compile_ingredient(
                                    value,
                                    &mut intern,
                                    &mut result.ingredients,
                                    &mut tags,
                                );
                            }
                        }
                    }
                    (
                        ItemStack::new(item, output.count),
                        ingredients,
                        width,
                        height,
                    )
                }
                CraftingRecipeTypes::CraftingShapeless {
                    ingredients,
                    result: output,
                    ..
                } if !ingredients.is_empty() && ingredients.len() <= 9 => {
                    let item =
                        ItemKind::from_name(output.id).expect("Crafting recipe result must exist");
                    let mut compiled = [EMPTY; 9];
                    for (slot, value) in compiled.iter_mut().zip(*ingredients) {
                        *slot = compile_ingredient(
                            value,
                            &mut intern,
                            &mut result.ingredients,
                            &mut tags,
                        );
                    }
                    (ItemStack::new(item, output.count), compiled, 0, 0)
                }
                _ => continue,
            };
            let index = result.recipes.len();
            let occupied = ingredients.iter().filter(|&&value| value != EMPTY).count();
            if width == 0 {
                result.shapeless[occupied].push(index);
            } else {
                result.shaped[height * 4 + width].push(index);
            }
            result.recipes.push(Recipe {
                output,
                ingredients,
                width,
                occupied,
            });
        }
        result
    })
}
pub(crate) fn initialize() {
    let _ = catalog();
    let _ = ItemKind::Block(crate::voxel::VoxelKind::Dirt).max_stack_size();
}
fn accepts(catalog: &Catalog, ingredient: u16, id: Option<u16>) -> bool {
    match id {
        None => ingredient == EMPTY,
        Some(id) => {
            ingredient != EMPTY
                && catalog.ingredients[ingredient as usize][id as usize / 64] & (1 << (id % 64))
                    != 0
        }
    }
}
/// Augmenting paths find a complete matching in polynomial time, even for overlapping tags.
fn assign(
    ingredient: usize,
    edges: &[u16; 9],
    assigned: &mut [Option<usize>; 9],
    seen: &mut u16,
) -> bool {
    for slot in 0..9 {
        let bit = 1 << slot;
        if edges[ingredient] & bit == 0 || *seen & bit != 0 {
            continue;
        }
        *seen |= bit;
        if assigned[slot].is_none_or(|previous| assign(previous, edges, assigned, seen)) {
            assigned[slot] = Some(ingredient);
            return true;
        }
    }
    false
}
fn find(grid: &CraftingGrid) -> Option<RecipeMatch> {
    let catalog = catalog();
    let mut ids = [None; 9];
    let mut occupied = 0u16;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (3, 3, 0, 0);
    for (index, stack) in grid.slots[..grid.active_len()].iter().enumerate() {
        if let Some(stack) = stack {
            ids[index] = Some(stack.item().registry().id);
            occupied |= 1 << index;
            min_x = min_x.min(index % grid.side);
            min_y = min_y.min(index / grid.side);
            max_x = max_x.max(index % grid.side + 1);
            max_y = max_y.max(index / grid.side + 1);
        }
    }
    if occupied == 0 {
        return None;
    }
    let count = occupied.count_ones() as usize;
    let (width, height) = (max_x - min_x, max_y - min_y);
    for &index in &catalog.shaped[height * 4 + width] {
        let recipe = &catalog.recipes[index];
        if recipe.occupied == count
            && [false, true].into_iter().any(|mirror| {
                (0..height).all(|y| {
                    (0..width).all(|x| {
                        let rx = if mirror { recipe.width - 1 - x } else { x };
                        accepts(
                            catalog,
                            recipe.ingredients[y * recipe.width + rx],
                            ids[(y + min_y) * grid.side + x + min_x],
                        )
                    })
                })
            })
        {
            return Some(RecipeMatch {
                recipe: index,
                consumed: occupied,
            });
        }
    }
    for &index in &catalog.shapeless[count] {
        let recipe = &catalog.recipes[index];
        let mut edges = [0; 9];
        for (ingredient, edge) in edges.iter_mut().enumerate().take(count) {
            for (slot, &id) in ids.iter().enumerate() {
                if id.is_some() && accepts(catalog, recipe.ingredients[ingredient], id) {
                    *edge |= 1 << slot;
                }
            }
        }
        let mut assigned = [None; 9];
        if (0..count).all(|ingredient| assign(ingredient, &edges, &mut assigned, &mut 0)) {
            return Some(RecipeMatch {
                recipe: index,
                consumed: occupied,
            });
        }
    }
    None
}

pub(crate) fn refresh_crafting(mut grids: Query<(Ref<CraftingGrid>, &mut CraftingResult)>) {
    grids.par_iter_mut().for_each(|(grid, mut result)| {
        if grid.is_changed() {
            result.set_if_neq(CraftingResult(grid.recipe().map(RecipeMatch::output)));
        }
    });
}

#[derive(Clone, Copy)]
pub(crate) enum CraftDestination {
    Cursor,
    Inventory,
    Hotbar(usize),
}

/// All mutations are staged on the stack; only successful complete recipes are committed.
pub(crate) fn craft(
    inventory: &mut PlayerInventory,
    grid: &mut CraftingGrid,
    cursor: &mut Option<ItemStack>,
    destination: CraftDestination,
) {
    let mut inventory_slots = inventory.snapshot();
    let mut current = *grid;
    let mut held = *cursor;
    let Some(mut matched) = current.recipe() else {
        return;
    };
    loop {
        let (mut next_slots, mut next_grid, mut next_held) = (inventory_slots, current, held);
        let output = matched.output();
        let placed = match destination {
            CraftDestination::Inventory => {
                slots::insert(&mut next_slots, output, 0..INVENTORY_SIZE).is_none()
            }
            CraftDestination::Cursor => merge(&mut next_held, output),
            CraftDestination::Hotbar(index) => merge(&mut next_slots[index], output),
        };
        if !placed {
            break;
        }
        let mut fits = true;
        for index in 0..current.active_len() {
            if matched.consumed & (1 << index) == 0 {
                continue;
            }
            let stack = next_grid.slots[index].expect("Matched ingredient");
            next_grid.slots[index] = stack.with_count(stack.count() - 1);
            if let Some(item) = stack.item().crafting_remainder() {
                let remainder = ItemStack::new(item, 1);
                if next_grid.slots[index].is_none() {
                    next_grid.slots[index] = Some(remainder);
                } else if slots::insert(&mut next_slots, remainder, 0..INVENTORY_SIZE).is_some() {
                    fits = false;
                    break;
                }
            }
        }
        if !fits {
            break;
        }
        let same_inputs = current
            .slots
            .iter()
            .zip(next_grid.slots)
            .all(|(a, b)| a.map(ItemStack::item) == b.map(ItemStack::item));
        (inventory_slots, current, held) = (next_slots, next_grid, next_held);
        if !matches!(destination, CraftDestination::Inventory) {
            break;
        }
        if !same_inputs {
            let Some(next) = current.recipe() else {
                break;
            };
            if next.recipe != matched.recipe {
                break;
            }
            matched = next;
        }
    }
    inventory.replace_slots(inventory_slots);
    *grid = current;
    *cursor = held;
}
fn merge(target: &mut Option<ItemStack>, output: ItemStack) -> bool {
    let count = u16::from(target.map_or(0, ItemStack::count)) + u16::from(output.count());
    if count > u16::from(output.max_stack_size())
        || target.is_some_and(|stack| !stack.can_merge(output))
    {
        return false;
    }
    *target = output.with_count(count as u8);
    true
}
