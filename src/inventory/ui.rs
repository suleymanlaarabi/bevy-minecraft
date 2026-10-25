use bevy::{
    picking::prelude::*,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use crate::{
    dropped_item::DroppedItemCommandsExt,
    game::GameState,
    inventory::{
        DragMode, DragState, HOTBAR_SIZE, INVENTORY_SIZE, InventoryCursor, InventoryState,
        ItemKind, PlayerInventory, icons::BlockIcons,
    },
    player::{Crosshair, Player},
};

const SLOT_SIZE: f32 = 48.0;
const SLOT_GAP: f32 = 4.0;
const ICON_SIZE: f32 = 32.0;

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

fn slot(index: usize, interactive: bool) -> impl Scene {
    let picking = (!interactive).then_some(bsn! { Pickable::IGNORE });
    let interactions = interactive.then_some(bsn! {
        on(slot_click)
        on(slot_over)
        on(slot_out)
        on(slot_drag_start)
        on(slot_drag_enter)
        on(slot_drag_over)
        on(slot_drag_end)
    });
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
        @picking
        @interactions
        Children [
            Node { width: px(ICON_SIZE), height: px(ICON_SIZE) }
            SlotIcon { index }
            ImageNode
            Visibility::Hidden
            Pickable::IGNORE
            --
            Text("")
            TextFont { font_size: FontSize::Px(14.0) }
            TextColor(Color::WHITE)
            Node {
                position_type: PositionType::Absolute,
                right: px(3),
                bottom: px(1),
            }
            SlotCount { index }
            Pickable::IGNORE
        ]
    }
}

fn slot_list(range: core::ops::Range<usize>, interactive: bool) -> impl SceneList {
    range
        .map(|index| slot(index, interactive))
        .collect::<Vec<_>>()
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
            Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                    HOTBAR_SIZE as u16, SLOT_SIZE,
                ),
                column_gap: px(SLOT_GAP),
                padding: UiRect::all(px(4)),
            }
            BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.55))
            Pickable::IGNORE
            Children [
                { slot_list(0..HOTBAR_SIZE, false) }
            ]
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
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(14),
                padding: UiRect::all(px(18)),
            }
            BackgroundColor(PANEL_BG)
            Children [
                Text("Inventory")
                TextFont {
                    font_size: FontSize::Px(20.0),
                }
                TextColor(Color::WHITE)
                Pickable::IGNORE
                --
                Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                        HOTBAR_SIZE as u16, SLOT_SIZE,
                    ),
                    grid_template_rows: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                        3_u16, SLOT_SIZE,
                    ),
                    column_gap: px(SLOT_GAP),
                    row_gap: px(SLOT_GAP),
                }
                Children [
                    { slot_list(HOTBAR_SIZE..INVENTORY_SIZE, true) }
                ]
                --
                Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(
                        HOTBAR_SIZE as u16, SLOT_SIZE,
                    ),
                    column_gap: px(SLOT_GAP),
                    margin: UiRect {
                        top: px(6),
                    },
                }
                Children [
                    { slot_list(0..HOTBAR_SIZE, true) }
                ]
            ]
            --
            Node {
                width: px(36),
                height: px(36),
                position_type: PositionType::Absolute,
            }
            CursorStackView
            Visibility::Hidden
            ImageNode
            Pickable::IGNORE
            GlobalZIndex(200)
            Children [
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
            ]
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
    mut players: Query<(&mut PlayerInventory, &GlobalTransform), With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
    state: Res<State<GameState>>,
    mut commands: Commands,
) {
    // Leaving the session discards the cursor through `reset_inventory_cursor`.
    if *state.get() != GameState::Game {
        return;
    }
    cursor.hovered_slot = None;
    cursor.drag = None;
    let Some(stack) = cursor.stack.take() else {
        return;
    };
    let Ok((mut inventory, transform)) = players.single_mut() else {
        // The session may already have despawned its player.
        return;
    };
    if let Some(remaining) =
        PlayerInventory::edit(&mut inventory, |inventory| inventory.insert(stack))
    {
        commands.spawn_dropped_item(
            remaining,
            transform.translation() + Vec3::Y * 0.5,
            Vec3::Y * 1.2,
        );
    }
}

/// Points `node` at the icon of `item`, leaving it untouched when already shown.
fn show_icon(node: &mut Mut<ImageNode>, icons: &BlockIcons, item: ItemKind) {
    let atlas = icons.atlas(item);
    let shown = node.image == *icons.image()
        && node
            .texture_atlas
            .as_ref()
            .is_some_and(|current| current.index == atlas.index);
    if !shown {
        node.image = icons.image().clone();
        node.texture_atlas = Some(atlas);
    }
}

pub(crate) fn refresh_slots(
    inventory: Single<Ref<PlayerInventory>, With<Player>>,
    new_views: Query<(), Added<InventorySlotView>>,
    block_icons: Res<BlockIcons>,
    mut icons: Query<(&SlotIcon, &mut ImageNode, &mut Visibility)>,
    mut counts: Query<(&SlotCount, &mut Text)>,
    mut slots: Query<(&InventorySlotView, &mut BorderColor)>,
) {
    if !inventory.is_changed() && new_views.is_empty() {
        return;
    }
    for (view, mut node, mut visibility) in &mut icons {
        match inventory.slot(view.index) {
            Some(stack) => {
                show_icon(&mut node, &block_icons, stack.item());
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }

    for (view, mut text) in &mut counts {
        let value = match inventory.slot(view.index) {
            Some(stack) if stack.count() > 1 => stack.count().to_string(),
            _ => String::new(),
        };
        text.set_if_neq(Text(value));
    }

    for (view, mut border) in &mut slots {
        let selected = view.index < HOTBAR_SIZE && view.index == inventory.selected_hotbar();

        border.set_if_neq(BorderColor::all(if selected {
            SELECTED_BORDER
        } else {
            SLOT_BORDER
        }));
    }
}

fn slot_over(
    event: On<PointerOver>,
    slots: Query<&InventorySlotView>,
    mut cursor: ResMut<InventoryCursor>,
) {
    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    cursor.hovered_slot = Some(slot.index);
}

fn slot_out(
    event: On<PointerOut>,
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
    mut event: On<PointerClick>,
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

    match event.button {
        PointerButton::Primary => {
            let shift =
                keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight);

            if shift && cursor.stack.is_none() {
                PlayerInventory::edit(&mut inventory, |inventory| inventory.quick_move(index));
                return;
            }

            if event.count >= 2 && cursor.stack.is_some() {
                PlayerInventory::edit(&mut inventory, |inventory| {
                    inventory.collect_matching(&mut cursor.stack)
                });
                return;
            }

            PlayerInventory::edit(&mut inventory, |inventory| {
                inventory.left_click(index, &mut cursor.stack)
            });
        }
        PointerButton::Secondary => {
            PlayerInventory::edit(&mut inventory, |inventory| {
                inventory.right_click(index, &mut cursor.stack)
            });
        }
        PointerButton::Middle => {}
    }
}

fn slot_drag_start(
    mut event: On<PointerDragStart>,
    slots: Query<&InventorySlotView>,
    mut inventory: Single<&mut PlayerInventory, With<Player>>,
    mut cursor: ResMut<InventoryCursor>,
) {
    event.propagate(false);

    let Ok(slot) = slots.get(event.entity) else {
        return;
    };

    let had_stack_before_drag = cursor.stack.is_some();

    let mode = match event.button {
        PointerButton::Primary => DragMode::Even,
        PointerButton::Secondary => DragMode::OneEach,
        PointerButton::Middle => return,
    };

    if !had_stack_before_drag {
        PlayerInventory::edit(&mut inventory, |inventory| {
            if event.button == PointerButton::Primary {
                inventory.left_click(slot.index, &mut cursor.stack);
            } else {
                inventory.right_click(slot.index, &mut cursor.stack);
            }
        });
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
    mut event: On<PointerDragEnter>,
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

    if event.dragged != drag.source_entity {
        return;
    }

    drag.visited[slot.index] = true;
}

fn slot_drag_over(
    mut event: On<PointerDragOver>,
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

    if event.dragged != drag.source_entity {
        return;
    }

    drag.visited[slot.index] = true;
}

fn slot_drag_end(
    mut event: On<PointerDragEnd>,
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

    PlayerInventory::edit(&mut inventory, |inventory| {
        inventory.distribute_drag(&mut cursor.stack, &drag.visited, drag.mode)
    });
}

pub(crate) fn follow_cursor_stack(
    window: Single<&Window, With<PrimaryWindow>>,
    cursor: Res<InventoryCursor>,
    block_icons: Res<BlockIcons>,
    view: Single<(&mut Node, &mut ImageNode, &mut Visibility), With<CursorStackView>>,
    mut count: Single<&mut Text, With<CursorStackCount>>,
) {
    let (mut node, mut image, mut visibility) = view.into_inner();

    let Some(stack) = cursor.stack else {
        visibility.set_if_neq(Visibility::Hidden);
        if !count.0.is_empty() {
            count.0.clear();
        }
        return;
    };

    let Some(position) = window.cursor_position() else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };

    visibility.set_if_neq(Visibility::Inherited);
    let left = px(position.x + 12.0);
    let top = px(position.y + 12.0);
    if node.left != left || node.top != top {
        node.left = left;
        node.top = top;
    }
    show_icon(&mut image, &block_icons, stack.item());
    let text = if stack.count() > 1 {
        stack.count().to_string()
    } else {
        String::new()
    };
    count.set_if_neq(Text(text));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct RecordedDrops(Vec<crate::inventory::ItemStack>);

    #[test]
    fn ending_session_clears_cursor_without_spawning_orphaned_drops() {
        use crate::{dropped_item::SpawnDroppedItem, inventory::ItemStack, voxel::VoxelKind};
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(State::new(GameState::Menu));
        world.init_resource::<InventoryCursor>();
        world.init_resource::<RecordedDrops>();
        world.add_observer(
            |event: On<SpawnDroppedItem>, mut drops: ResMut<RecordedDrops>| {
                drops.0.push(event.stack)
            },
        );
        world.spawn(Player);
        world.resource_mut::<InventoryCursor>().stack =
            Some(ItemStack::new(ItemKind::Block(VoxelKind::Dirt), 1));
        world.run_system_once(return_cursor_stack).unwrap();
        world
            .run_system_once(super::super::reset_inventory_cursor)
            .unwrap();
        assert!(world.resource::<InventoryCursor>().stack.is_none());
        assert!(world.resource::<RecordedDrops>().0.is_empty());
    }

    #[test]
    fn closing_full_inventory_drops_remainder_without_losing_items() {
        use crate::{dropped_item::SpawnDroppedItem, inventory::ItemStack, voxel::VoxelKind};
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(State::new(GameState::Game));
        world.init_resource::<InventoryCursor>();
        world.init_resource::<RecordedDrops>();
        world.add_observer(
            |event: On<SpawnDroppedItem>, mut drops: ResMut<RecordedDrops>| {
                drops.0.push(event.stack)
            },
        );
        let mut inventory = PlayerInventory::default();
        inventory.insert(ItemStack::new(ItemKind::Block(VoxelKind::Dirt), 62));
        for _ in 1..INVENTORY_SIZE {
            assert!(
                inventory
                    .insert(ItemStack::new(ItemKind::Block(VoxelKind::Stone), 64))
                    .is_none()
            );
        }
        let mut held = None;
        inventory.left_click(0, &mut held);
        inventory.insert(ItemStack::new(ItemKind::Block(VoxelKind::Wood), 1));
        let player = world
            .spawn((
                Player,
                inventory,
                GlobalTransform::from_translation(Vec3::Y),
            ))
            .id();
        world.resource_mut::<InventoryCursor>().stack = held;
        let before: usize = (0..INVENTORY_SIZE)
            .filter_map(|i| world.get::<PlayerInventory>(player).unwrap().slot(i))
            .map(|s| s.count() as usize)
            .sum::<usize>()
            + held.unwrap().count() as usize;
        world.run_system_once(return_cursor_stack).unwrap();
        assert!(world.resource::<InventoryCursor>().stack.is_none());
        assert_eq!(
            world.resource::<RecordedDrops>().0.as_slice(),
            &[held.unwrap()]
        );
        let after: usize = (0..INVENTORY_SIZE)
            .filter_map(|i| world.get::<PlayerInventory>(player).unwrap().slot(i))
            .map(|s| s.count() as usize)
            .sum::<usize>()
            + world
                .resource::<RecordedDrops>()
                .0
                .iter()
                .map(|s| s.count() as usize)
                .sum::<usize>();
        assert_eq!(before, after);
    }

    #[test]
    fn slots_initialize_when_opened_and_remain_unchanged_at_rest() {
        let mut app = App::new();
        app.init_resource::<BlockIcons>()
            .add_systems(Update, refresh_slots);
        app.world_mut().spawn(Player);
        let view = app
            .world_mut()
            .spawn((
                InventorySlotView { index: 0 },
                BorderColor::all(Color::NONE),
            ))
            .id();
        let icon = app
            .world_mut()
            .spawn((
                SlotIcon { index: 0 },
                ImageNode::default(),
                Visibility::Hidden,
            ))
            .id();
        let count = app
            .world_mut()
            .spawn((SlotCount { index: 0 }, Text::new("")))
            .id();
        app.update();
        assert_eq!(app.world().get::<Text>(count).unwrap().0, "2");
        let text_tick = app
            .world()
            .entity(count)
            .get_change_ticks::<Text>()
            .unwrap()
            .changed;
        let icon_tick = app
            .world()
            .entity(icon)
            .get_change_ticks::<ImageNode>()
            .unwrap()
            .changed;
        let border_tick = app
            .world()
            .entity(view)
            .get_change_ticks::<BorderColor>()
            .unwrap()
            .changed;
        app.update();
        assert_eq!(
            app.world()
                .entity(count)
                .get_change_ticks::<Text>()
                .unwrap()
                .changed,
            text_tick
        );
        assert_eq!(
            app.world()
                .entity(icon)
                .get_change_ticks::<ImageNode>()
                .unwrap()
                .changed,
            icon_tick
        );
        assert_eq!(
            app.world()
                .entity(view)
                .get_change_ticks::<BorderColor>()
                .unwrap()
                .changed,
            border_tick
        );
        let new_count = app
            .world_mut()
            .spawn((SlotCount { index: 0 }, Text::new("")))
            .id();
        app.world_mut().spawn((
            InventorySlotView { index: 0 },
            BorderColor::all(Color::NONE),
        ));
        app.update();
        assert_eq!(app.world().get::<Text>(new_count).unwrap().0, "2");
        assert_eq!(
            app.world()
                .entity(count)
                .get_change_ticks::<Text>()
                .unwrap()
                .changed,
            text_tick
        );
    }

    #[test]
    fn inventory_scenes_preserve_all_slot_indices() {
        for (scene, expected_count) in [
            (Box::new(hotbar_hud_scene()) as Box<dyn Scene>, HOTBAR_SIZE),
            (Box::new(inventory_overlay_scene()), INVENTORY_SIZE),
        ] {
            let mut app = App::new();
            app.add_plugins((
                TaskPoolPlugin::default(),
                AssetPlugin::default(),
                bevy::scene::ScenePlugin,
            ));
            let world = app.world_mut();
            world.spawn_scene(scene).unwrap();
            let mut indices: Vec<_> = world
                .query::<&InventorySlotView>()
                .iter(world)
                .map(|slot| slot.index)
                .collect();
            indices.sort_unstable();
            assert_eq!(indices, (0..expected_count).collect::<Vec<_>>());
        }
    }
}
