pub(crate) mod access;
pub(crate) mod catalog;
pub(crate) mod crafting;
pub(crate) mod icons;
mod input;
pub(crate) mod mining;
mod model;
pub(crate) mod slots;
mod ui;
pub(crate) use crafting::{CraftingGrid, CraftingResult};
pub(crate) use model::ITEM_ID_LIMIT;

use bevy::input::common_conditions::input_just_pressed;
use bevy::prelude::*;
use input::{handle_hotbar_scroll, handle_number_keys, handle_swap_offhand};

use crate::game::GameState;

pub use model::{
    ARMOR_START, BLOCK_STACK_SIZE, CRAFT_RESULT, CRAFT_START, ConsumableContents, DragMode,
    HOTBAR_SIZE, HotbarSelection, INVENTORY_SIZE, ItemKind, ItemStack, MAIN_INVENTORY_SIZE,
    OFFHAND_SLOT, PlayerInventory, SLOT_COUNT,
};

#[derive(SubStates, Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[source(GameState = GameState::Game)]
pub enum InventoryState {
    #[default]
    Closed,
    Open,
}

#[derive(Clone, Debug)]
pub(crate) struct DragState {
    pub source_entity: Entity,
    pub mode: DragMode,
    pub visited: [bool; access::MAX_VIEW_SLOTS],
}

#[derive(Resource, Default)]
pub(crate) struct InventoryCursor {
    pub stack: Option<ItemStack>,
    pub hovered_slot: Option<usize>,
    pub drag: Option<DragState>,
}

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.add_sub_state::<InventoryState>()
            .init_resource::<InventoryCursor>()
            .add_message::<access::InventoryAction>()
            .init_resource::<icons::BlockIcons>()
            .add_systems(PreStartup, catalog::initialize)
            .add_systems(
                Startup,
                (
                    icons::setup_block_icons,
                    crafting::initialize,
                    mining::initialize,
                ),
            )
            .add_systems(
                Update,
                icons::render_block_icons.run_if(
                    resource_exists::<crate::voxel::VoxelAssets>
                        .and_then(resource_exists::<crate::voxel::TexturePack>),
                ),
            )
            .add_systems(OnExit(GameState::Game), reset_inventory_cursor)
            .add_systems(
                OnEnter(crate::world::WorldSessionPhase::Playing),
                ui::return_cursor_stack,
            )
            .add_systems(OnEnter(GameState::Game), ui::spawn_hotbar_hud)
            .add_systems(
                OnEnter(InventoryState::Open),
                (ui::hide_game_hud, ui::spawn_inventory_overlay).chain(),
            )
            .add_systems(
                OnExit(InventoryState::Open),
                (ui::return_cursor_stack, ui::show_game_hud).chain(),
            )
            .add_systems(
                Update,
                toggle_inventory
                    .run_if(crate::world::playing)
                    .run_if(crate::survival::player_alive)
                    .run_if(crate::devtools::panel_closed)
                    .run_if(input_just_pressed(KeyCode::KeyE)),
            )
            .add_systems(
                Update,
                (
                    handle_number_keys,
                    handle_hotbar_scroll.run_if(in_state(InventoryState::Closed)),
                    handle_swap_offhand
                        .run_if(in_state(InventoryState::Closed))
                        .run_if(input_just_pressed(KeyCode::KeyF)),
                )
                    .chain()
                    .run_if(crate::devtools::panel_closed)
                    .run_if(crate::survival::player_alive)
                    .run_if(crate::world::playing),
            )
            .add_systems(
                PostUpdate,
                crafting::refresh_crafting
                    .in_set(crate::block_behavior::GameplaySystems::Derived)
                    .run_if(crate::world::playing),
            )
            .add_systems(
                PostUpdate,
                access::apply_actions
                    .in_set(crate::block_behavior::GameplaySystems::Inventory)
                    .run_if(crate::world::playing),
            )
            .add_systems(
                PostUpdate,
                (
                    ui::bind_slots,
                    ui::refresh_slots,
                    ui::follow_cursor_stack.run_if(in_state(InventoryState::Open)),
                )
                    .chain()
                    .in_set(crate::block_behavior::GameplaySystems::Presentation)
                    .run_if(crate::world::playing),
            );
    }
}

fn toggle_inventory(
    state: Res<State<InventoryState>>,
    mut next_state: ResMut<NextState<InventoryState>>,
) {
    next_state.set(match state.get() {
        InventoryState::Closed => InventoryState::Open,
        InventoryState::Open => InventoryState::Closed,
    });
}

fn reset_inventory_cursor(
    mut cursor: ResMut<InventoryCursor>,
    mut actions: ResMut<Messages<access::InventoryAction>>,
) {
    actions.clear();
    *cursor = InventoryCursor::default();
}
