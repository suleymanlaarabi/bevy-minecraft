use bevy::{
    picking::prelude::*,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use crate::{
    inventory::{
        DragMode, DragState, HOTBAR_SIZE, INVENTORY_SIZE, InventoryCursor, InventoryState,
        ItemKind, PlayerInventory,
    },
    player::{Crosshair, Player},
};

const SLOT_SIZE: f32 = 48.0;
const SLOT_GAP: f32 = 4.0;

const SLOT_BG: Color = Color::srgba(0.08, 0.08, 0.09, 0.94);
const SLOT_BORDER: Color = Color::srgba(0.35, 0.35, 0.38, 1.0);
const SELECTED_BORDER: Color = Color::WHITE;
const PANEL_BG: Color = Color::srgba(0.10, 0.10, 0.11, 0.97);

#[derive(Component, Clone, Default)]
pub(crate) struct HotbarHud;

#[derive(Component, Clone, Default)]
struct InventoryOverlay;

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct InventorySlotView {
    index: usize,
}

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct SlotIcon {
    index: usize,
}

#[derive(Component, Clone, Copy, Default)]
pub(crate) struct SlotCount {
    index: usize,
}

#[derive(Component, Clone, Default)]
pub(crate) struct CursorStackView;

#[derive(Component, Clone, Default)]
pub(crate) struct CursorStackCount;

fn hud_slot(index: usize) -> impl Scene {
    bsn! {
        Node {
            width: px(SLOT_SIZE),
            height: px(SLOT_SIZE),
            border: UiRect::all(px(2)),
            position_type: PositionType::Relative,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        InventorySlotView { index }
        BackgroundColor(SLOT_BG)
        BorderColor::all(SLOT_BORDER)
        Pickable::IGNORE
        Children [
            (
                Node {
                    width: px(30),
                    height: px(30),
                }
                SlotIcon { index }
                BackgroundColor(Color::NONE)
                Visibility::Hidden
                Pickable::IGNORE
            ),
            (
                Text("")
                TextFont {
                    font_size: FontSize::Px(14.0),
                }
                TextColor(Color::WHITE)
                Node {
                    position_type: PositionType::Absolute,
                    right: px(3),
                    bottom: px(1),
                }
                SlotCount { index }
                Pickable::IGNORE
            )
        ]
    }
}

fn inventory_slot(index: usize) -> impl Scene {
    bsn! {
        Node {
            width: px(SLOT_SIZE),
            height: px(SLOT_SIZE),
            border: UiRect::all(px(2)),
            position_type: PositionType::Relative,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        InventorySlotView { index }
        BackgroundColor(SLOT_BG)
        BorderColor::all(SLOT_BORDER)

        on(slot_click)
        on(slot_over)
        on(slot_out)
        on(slot_drag_start)
        on(slot_drag_enter)
        on(slot_drag_over)
        on(slot_drag_end)

        Children [
            (
                Node {
                    width: px(30),
                    height: px(30),
                }
                SlotIcon { index }
                BackgroundColor(Color::NONE)
                Visibility::Hidden
                Pickable::IGNORE
            ),
            (
                Text("")
                TextFont {
                    font_size: FontSize::Px(14.0),
                }
                TextColor(Color::WHITE)
                Node {
                    position_type: PositionType::Absolute,
                    right: px(3),
                    bottom: px(1),
                }
                SlotCount { index }
                Pickable::IGNORE
            )
        ]
    }
}

fn hud_hotbar_slots() -> impl SceneList {
    bsn_list![
        hud_slot(0),
        hud_slot(1),
        hud_slot(2),
        hud_slot(3),
        hud_slot(4),
        hud_slot(5),
        hud_slot(6),
        hud_slot(7),
        hud_slot(8),
    ]
}

fn inventory_hotbar_slots() -> impl SceneList {
    bsn_list![
        inventory_slot(0),
        inventory_slot(1),
        inventory_slot(2),
        inventory_slot(3),
        inventory_slot(4),
        inventory_slot(5),
        inventory_slot(6),
        inventory_slot(7),
        inventory_slot(8),
    ]
}

fn main_inventory_slots() -> impl SceneList {
    bsn_list![
        inventory_slot(9),
        inventory_slot(10),
        inventory_slot(11),
        inventory_slot(12),
        inventory_slot(13),
        inventory_slot(14),
        inventory_slot(15),
        inventory_slot(16),
        inventory_slot(17),
        inventory_slot(18),
        inventory_slot(19),
        inventory_slot(20),
        inventory_slot(21),
        inventory_slot(22),
        inventory_slot(23),
        inventory_slot(24),
        inventory_slot(25),
        inventory_slot(26),
        inventory_slot(27),
        inventory_slot(28),
        inventory_slot(29),
        inventory_slot(30),
        inventory_slot(31),
        inventory_slot(32),
        inventory_slot(33),
        inventory_slot(34),
        inventory_slot(35),
    ]
}

fn hotbar_hud_scene() -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::End,
            padding: UiRect {
                bottom: px(24),
            },
        }
        HotbarHud
        Pickable::IGNORE
        DespawnOnExit::<GameState>(GameState::Game)
        Children [
            (
                Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                        9_u16, SLOT_SIZE,
                    ),
                    column_gap: px(SLOT_GAP),
                    padding: UiRect::all(px(4)),
                }
                BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.55))
                Pickable::IGNORE
                Children [
                    { hud_hotbar_slots() }
                ]
            )
        ]
    }
}

pub(crate) fn spawn_hotbar_hud(mut commands: Commands) {
    commands.spawn_scene(hotbar_hud_scene());
}

fn inventory_overlay_scene() -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        }
        InventoryOverlay
        GlobalZIndex(100)
        Pickable {
            should_block_lower: true,
            is_hoverable: false,
        }
        DespawnOnExit::<InventoryState>(InventoryState::Open)
        Children [
            (
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(14),
                    padding: UiRect::all(px(18)),
                }
                BackgroundColor(PANEL_BG)
                Children [
                    (
                        Text("Inventory")
                        TextFont {
                            font_size: FontSize::Px(20.0),
                        }
                        TextColor(Color::WHITE)
                        Pickable::IGNORE
                    ),
                    (
                        Node {
                            display: Display::Grid,
                            grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                                9_u16, SLOT_SIZE,
                            ),
                            grid_template_rows: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                                3_u16, SLOT_SIZE,
                            ),
                            column_gap: px(SLOT_GAP),
                            row_gap: px(SLOT_GAP),
                        }
                        Children [
                            { main_inventory_slots() }
                        ]
                    ),
                    (
                        Node {
                            display: Display::Grid,
                            grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                                9_u16, SLOT_SIZE,
                            ),
                            column_gap: px(SLOT_GAP),
                            margin: UiRect {
                                top: px(6),
                            },
                        }
                        Children [
                            { inventory_hotbar_slots() }
                        ]
                    )
                ]
            ),
            (
                Node {
                    width: px(36),
                    height: px(36),
                    position_type: PositionType::Absolute,
                }
                CursorStackView
                Visibility::Hidden
                BackgroundColor(Color::NONE)
                Pickable::IGNORE
                GlobalZIndex(200)
                Children [
                    (
                        Text("")
                        TextFont {
                            font_size: FontSize::Px(14.0),
                        }
                        TextColor(Color::WHITE)
                        Node {
                            position_type: PositionType::Absolute,
                            right: px(1),
                            bottom: px(0),
                        }
                        CursorStackCount
                        Pickable::IGNORE
                    )
                ]
            )
        ]
    }
}

pub(crate) fn spawn_inventory_overlay(mut commands: Commands) {
    commands.spawn_scene(inventory_overlay_scene());
}

pub(crate) fn unlock_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut cursor in &mut cursors {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

pub(crate) fn lock_cursor(mut cursors: Query<&mut CursorOptions>) {
    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
}

pub(crate) fn hide_game_hud(
    mut hotbar: Query<&mut Visibility, With<HotbarHud>>,
    mut crosshair: Query<&mut Visibility, (With<Crosshair>, Without<HotbarHud>)>,
) {
    for mut visibility in &mut hotbar {
        *visibility = Visibility::Hidden;
    }

    for mut visibility in &mut crosshair {
        *visibility = Visibility::Hidden;
    }
}

pub(crate) fn show_game_hud(
    mut hotbar: Query<&mut Visibility, With<HotbarHud>>,
    mut crosshair: Query<&mut Visibility, (With<Crosshair>, Without<HotbarHud>)>,
) {
    for mut visibility in &mut hotbar {
        *visibility = Visibility::Inherited;
    }

    for mut visibility in &mut crosshair {
        *visibility = Visibility::Inherited;
    }
}

pub(crate) fn return_cursor_stack(
    mut players: Query<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    cursor.hovered_slot = None;
    cursor.drag = None;

    let Some(stack) = cursor.stack.take() else {
        return;
    };

    let Ok(mut inventory) = players.single_mut() else {
        cursor.stack = Some(stack);
        return;
    };

    cursor.stack = inventory.insert(stack);

    debug_assert!(
        cursor.stack.is_none(),
        "a stack taken from the player inventory must fit back into it"
    );
}

fn item_color(item: ItemKind) -> Color {
    match item {
        ItemKind::Block(kind) => {
            let [r, g, b, a] = kind.color();
            Color::srgba(r, g, b, a)
        }
    }
}

pub(crate) fn refresh_slots(
    inventory: Single<&PlayerInventory, With<Player>>,
    mut icons: Query<(&SlotIcon, &mut BackgroundColor, &mut Visibility)>,
    mut counts: Query<(&SlotCount, &mut Text)>,
    mut slots: Query<(&InventorySlotView, &mut BorderColor)>,
) {
    for (view, mut color, mut visibility) in &mut icons {
        match inventory.slot(view.index) {
            Some(stack) => {
                color.0 = item_color(stack.item());
                *visibility = Visibility::Inherited;
            }
            None => {
                color.0 = Color::NONE;
                *visibility = Visibility::Hidden;
            }
        }
    }

    for (view, mut text) in &mut counts {
        text.0 = match inventory.slot(view.index) {
            Some(stack) if stack.count() > 1 => stack.count().to_string(),
            _ => String::new(),
        };
    }

    for (view, mut border) in &mut slots {
        let selected = view.index < HOTBAR_SIZE && view.index == inventory.selected_hotbar();

        *border = BorderColor::all(if selected {
            SELECTED_BORDER
        } else {
            SLOT_BORDER
        });
    }
}

fn slot_over(
    event: On<Pointer<Over>>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    cursor.hovered_slot = Some(slot.index);
}

fn slot_out(
    event: On<Pointer<Out>>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    if cursor.hovered_slot == Some(slot.index) {
        cursor.hovered_slot = None;
    }
}

fn slot_click(
    mut event: On<Pointer<Click>>,
    slots: Query<&InventorySlotView>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    if cursor.drag.is_some() {
        return;
    }

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let index = slot.index;

    match event.event.button {
        PointerButton::Primary => {
            let shift =
                keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight);

            if shift && cursor.stack.is_none() {
                inventory.quick_move(index);
                return;
            }

            if event.event.count >= 2 && cursor.stack.is_some() {
                inventory.collect_matching(&mut cursor.stack);
                return;
            }

            inventory.left_click(index, &mut cursor.stack);
        }
        PointerButton::Secondary => {
            inventory.right_click(index, &mut cursor.stack);
        }
        PointerButton::Middle => {}
    }
}

fn slot_drag_start(
    mut event: On<Pointer<DragStart>>,
    slots: Query<&InventorySlotView>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let had_stack_before_drag = cursor.stack.is_some();

    let mode = match event.event.button {
        PointerButton::Primary => DragMode::Even,
        PointerButton::Secondary => DragMode::OneEach,
        PointerButton::Middle => return,
    };

    if !had_stack_before_drag {
        match event.event.button {
            PointerButton::Primary => inventory.left_click(slot.index, &mut cursor.stack),
            PointerButton::Secondary => inventory.right_click(slot.index, &mut cursor.stack),
            PointerButton::Middle => return,
        }
    }

    if cursor.stack.is_none() {
        return;
    }

    let mut visited = [false; INVENTORY_SIZE];

    if had_stack_before_drag {
        visited[slot.index] = true;
    }

    cursor.drag = Some(DragState {
        source_entity: event.entity,
        mode,
        visited,
    });
}

fn slot_drag_enter(
    mut event: On<Pointer<DragEnter>>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let Some(drag) = cursor.drag.as_mut() else {
        return;
    };

    if event.event.dragged != drag.source_entity {
        return;
    }

    drag.visited[slot.index] = true;
}

fn slot_drag_over(
    mut event: On<Pointer<DragOver>>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let Some(drag) = cursor.drag.as_mut() else {
        return;
    };

    if event.event.dragged != drag.source_entity {
        return;
    }

    drag.visited[slot.index] = true;
}

fn slot_drag_end(
    mut event: On<Pointer<DragEnd>>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Some(drag) = cursor.drag.take() else {
        return;
    };

    if drag.source_entity != event.entity {
        cursor.drag = Some(drag);
        return;
    }

    inventory.distribute_drag(&mut cursor.stack, &drag.visited, drag.mode);
}

pub(crate) fn follow_cursor_stack(
    window: Single<&Window, With<PrimaryWindow>>,
    cursor: Res<InventoryCursor>,
    view: Single<(&mut Node, &mut BackgroundColor, &mut Visibility), With<CursorStackView>>,
    mut count: Single<&mut Text, With<CursorStackCount>>,
) {
    let (mut node, mut color, mut visibility) = view.into_inner();

    let Some(stack) = cursor.stack else {
        *visibility = Visibility::Hidden;
        count.0.clear();
        return;
    };

    let Some(position) = window.cursor_position() else {
        *visibility = Visibility::Hidden;
        return;
    };

    *visibility = Visibility::Inherited;

    node.left = px(position.x + 12.0);
    node.top = px(position.y + 12.0);

    color.0 = item_color(stack.item());

    count.0 = if stack.count() > 1 {
        stack.count().to_string()
    } else {
        String::new()
    };
}
