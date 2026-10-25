mod crafting;
pub(crate) mod icons;
mod input;
mod model;
mod ui;

use bevy::prelude::*;
use input::{handle_hotbar_scroll, handle_number_keys, handle_swap_offhand};

use crate::{game::GameState, player::Player};

pub use model::{
    ARMOR_START, BLOCK_STACK_SIZE, CRAFT_RESULT, CRAFT_START, DragMode, HOTBAR_SIZE,
    INVENTORY_SIZE, ItemKind, ItemStack, MAIN_INVENTORY_SIZE, OFFHAND_SLOT, PlayerInventory,
    SLOT_COUNT,
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
    pub visited: [bool; SLOT_COUNT],
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
            .init_resource::<icons::BlockIcons>()
            .add_systems(Startup, (icons::setup_block_icons, crafting::initialize))
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
                (
                    ui::unlock_cursor,
                    ui::hide_game_hud,
                    ui::spawn_inventory_overlay,
                )
                    .chain(),
            )
            .add_systems(
                OnExit(InventoryState::Open),
                (ui::return_cursor_stack, ui::lock_cursor, ui::show_game_hud).chain(),
            )
            .add_systems(
                Update,
                toggle_inventory
                    .run_if(crate::world::playing)
                    // Nothing to open while the world is still loading.
                    .run_if(any_with_component::<Player>),
            )
            .add_systems(
                Update,
                (
                    handle_number_keys,
                    handle_hotbar_scroll,
                    handle_swap_offhand,
                    ui::refresh_slots,
                )
                    .chain()
                    .run_if(crate::world::playing),
            )
            .add_systems(
                Update,
                ui::follow_cursor_stack.run_if(in_state(InventoryState::Open)),
            );
    }
}

fn toggle_inventory(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<InventoryState>>,
    mut next_state: ResMut<NextState<InventoryState>>,
) {
    if !keyboard.just_pressed(KeyCode::KeyE) {
        return;
    }

    next_state.set(match state.get() {
        InventoryState::Closed => InventoryState::Open,
        InventoryState::Open => InventoryState::Closed,
    });
}

fn reset_inventory_cursor(mut cursor: ResMut<InventoryCursor>) {
    *cursor = InventoryCursor::default();
}
