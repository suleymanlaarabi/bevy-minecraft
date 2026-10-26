use super::interactions::{
    slot_click, slot_drag_end, slot_drag_enter, slot_drag_over, slot_drag_start, slot_out,
    slot_over, slot_press,
};
use super::{
    CursorStackCount, CursorStackView, EmptySlotLabel, HotbarHud, ICON_SIZE, InventoryOverlay,
    InventorySlotView, PANEL_BG, SLOT_BG, SLOT_BORDER, SLOT_GAP, SLOT_SIZE, SlotCount, SlotIcon,
    empty_slot_label,
};
use crate::{
    game::GameState,
    inventory::{
        ARMOR_START, CRAFT_RESULT, CRAFT_START, HOTBAR_SIZE, INVENTORY_SIZE, InventoryState,
        OFFHAND_SLOT, SLOT_COUNT,
    },
};
use bevy::{picking::prelude::Pickable, prelude::*};

fn slot(index: usize, interactive: bool) -> impl Scene {
    let picking = (!interactive).then_some(bsn! { Pickable::IGNORE });
    let interactions = interactive.then_some(bsn! {
        on(slot_press)
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
            Text({empty_slot_label(index)})
            EmptySlotLabel
            TextFont { font_size: FontSize::Px(10.0) }
            TextColor(Color::srgb(0.48,0.48,0.52))
            Node { position_type: PositionType::Absolute }
            Pickable::IGNORE
            --
            Node { width: px(ICON_SIZE), height: px(ICON_SIZE) }
            SlotIcon
            ImageNode
            Visibility::Hidden
            Pickable::IGNORE
            --
            @{ count_text(bsn! { SlotCount }, 3.0, 1.0) }
        ]
    }
}

fn slot_list(range: core::ops::Range<usize>, interactive: bool) -> impl SceneList {
    range
        .map(|index| slot(index, interactive))
        .collect::<Vec<_>>()
}

fn count_text(marker: impl Scene, right: f32, bottom: f32) -> impl Scene {
    bsn! {
        Text("")
        TextFont { font_size: FontSize::Px(14.0) }
        TextColor(Color::WHITE)
        Node { position_type: PositionType::Absolute, right: px(right), bottom: px(bottom) }
        @marker
        Pickable::IGNORE
    }
}

fn cursor_stack_scene() -> impl Scene {
    bsn! {
        Node { width: px(ICON_SIZE), height: px(ICON_SIZE), position_type: PositionType::Absolute }
        CursorStackView
        Visibility::Hidden
        ImageNode
        Pickable::IGNORE
        GlobalZIndex(200)
        Children [ @{ count_text(bsn! { CursorStackCount }, 1.0, 0.0) } ]
    }
}

fn slot_grid(range: core::ops::Range<usize>, top_margin: f32) -> impl Scene {
    bsn! {
        Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(HOTBAR_SIZE as u16, SLOT_SIZE),
            column_gap: px(SLOT_GAP), row_gap: px(SLOT_GAP),
            margin: UiRect { top: px(top_margin) },
        }
        Children [ { slot_list(range, true) } ]
    }
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

fn overlay_scene(title: &'static str, content: impl SceneList) -> impl Scene {
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
        Pickable { should_block_lower: true, is_hoverable: false }
        DespawnOnExit::<InventoryState>(InventoryState::Open)
        Children [
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(14),
                padding: UiRect::all(px(18)),
            }
            BackgroundColor(PANEL_BG)
            Children [
                Text(title)
                TextFont { font_size: FontSize::Px(20.0) }
                TextColor(Color::WHITE)
                Pickable::IGNORE
                --
                { content }
                --
                @{ slot_grid(HOTBAR_SIZE..INVENTORY_SIZE, 0.0) }
                --
                @{ slot_grid(0..HOTBAR_SIZE, 6.0) }
            ]
            --
            @{ cursor_stack_scene() }
        ]
    }
}
fn inventory_overlay_scene(preview: Handle<Image>) -> impl Scene {
    overlay_scene(
        "Inventory",
        bsn! {
            Node { column_gap: px(10), height: px(204) }
            Children [
                Node { flex_direction: FlexDirection::Column, row_gap: px(SLOT_GAP) }
                Children [ { slot_list(ARMOR_START..OFFHAND_SLOT, true) } ]
                --
                Node { width: px(136), height: px(204) }
                ImageNode { image: preview }
                crate::player::skin::SkinPortrait
                Pickable::IGNORE
                --
                Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, row_gap: px(10) }
                Children [
                    Text("Crafting")
                    TextFont { font_size: FontSize::Px(15.0) }
                    TextColor(Color::WHITE)
                    Pickable::IGNORE
                    --
                    Node { align_items: AlignItems::Center, column_gap: px(8) }
                    Children [
                        Node {
                            display: Display::Grid,
                            grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(2_u16, SLOT_SIZE),
                            column_gap: px(SLOT_GAP), row_gap: px(SLOT_GAP),
                        }
                        Children [ { slot_list(CRAFT_START..SLOT_COUNT, true) } ]
                        --
                        Text("→")
                        TextFont { font_size: FontSize::Px(20.0) }
                        TextColor(Color::srgb(0.65,0.65,0.68))
                        Pickable::IGNORE
                        --
                        { slot_list(CRAFT_RESULT..CRAFT_RESULT+1, true) }
                    ]
                    --
                    Node { align_items: AlignItems::Center, column_gap: px(10) }
                    Children [
                        { slot_list(OFFHAND_SLOT..OFFHAND_SLOT+1, true) }
                        --
                        Text("Offhand")
                        TextFont { font_size: FontSize::Px(13.0) }
                        TextColor(Color::srgb(0.65,0.65,0.68))
                        Pickable::IGNORE
                    ]
                ]
            ]
        },
    )
}

pub(crate) fn spawn_inventory_overlay(
    mut commands: Commands,
    assets: Res<crate::player::skin::SkinAssets>,
    partners: Query<&crate::block_behavior::containers::ChestPartners>,
    sessions: Query<
        &crate::block_behavior::containers::ViewingContainer,
        With<crate::player::Player>,
    >,
) {
    if let Ok(session) = sessions.single() {
        let count = 27 * (1 + partners.get(session.0).map_or(0, |partners| partners.len()));
        commands.spawn_scene(container_overlay_scene(count));
    } else {
        commands.spawn_scene(inventory_overlay_scene(assets.preview.clone()));
    }
}

fn container_overlay_scene(count: usize) -> impl Scene {
    let start = crate::inventory::access::CONTAINER_START;
    let title = if count > crate::block_behavior::containers::CONTAINER_SIZE {
        "Large Chest"
    } else {
        "Chest"
    };
    overlay_scene(
        title,
        (
            bsn! { @{ slot_grid(start..start + count, 0.0) } },
            bsn! {
                Text("Inventory")
                TextFont { font_size: FontSize::Px(15.0) }
                TextColor(Color::WHITE)
                Pickable::IGNORE
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_scenes_preserve_all_slot_indices() {
        for (scene, expected_count) in [
            (Box::new(hotbar_hud_scene()) as Box<dyn Scene>, HOTBAR_SIZE),
            (
                Box::new(inventory_overlay_scene(Handle::default())),
                SLOT_COUNT + 1,
            ),
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
