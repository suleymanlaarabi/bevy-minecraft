use super::interactions::{
    slot_click, slot_drag_end, slot_drag_enter, slot_drag_over, slot_drag_start, slot_out,
    slot_over, slot_press,
};
use super::{
    CursorStackView, EmptySlotLabel, ICON_SIZE, InventoryOverlay, InventorySlotView, PANEL_BG,
    SLOT_BG, SLOT_BORDER, SLOT_GAP, SLOT_SIZE, SlotCount, SlotIcon, empty_slot_label,
};
use crate::{
    game::GameState,
    inventory::{
        ARMOR_START, CRAFT_RESULT, CRAFT_START, HOTBAR_SIZE, INVENTORY_SIZE, InventoryState,
        OFFHAND_SLOT,
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
            @{ stack_icon() }
            --
            @{ count_text(3.0, 1.0) }
            --
            Node { position_type: PositionType::Absolute, left: px(4), bottom: px(3), height: px(3), width: px(0) }
            super::SlotDurability
            BackgroundColor(Color::srgb(0.25, 0.85, 0.25))
            Visibility::Hidden
            Pickable::IGNORE
        ]
    }
}

fn slot_list(range: core::ops::Range<usize>, interactive: bool) -> impl SceneList {
    range
        .map(|index| slot(index, interactive))
        .collect::<Vec<_>>()
}

fn count_text(right: f32, bottom: f32) -> impl Scene {
    bsn! {
        Text("")
        TextFont { font_size: FontSize::Px(14.0) }
        TextColor(Color::WHITE)
        Node { position_type: PositionType::Absolute, right: px(right), bottom: px(bottom) }
        SlotCount
        Pickable::IGNORE
    }
}

fn stack_icon() -> impl Scene {
    bsn! {
        Node { width: px(ICON_SIZE), height: px(ICON_SIZE) }
        SlotIcon
        ImageNode
        Visibility::Hidden
        Pickable::IGNORE
    }
}

fn cursor_stack_scene() -> impl Scene {
    bsn! {
        Node { width: px(ICON_SIZE), height: px(ICON_SIZE), position_type: PositionType::Absolute }
        CursorStackView
        Visibility::Hidden
        Pickable::IGNORE
        GlobalZIndex(200)
        Children [
            @{ stack_icon() }
            --
            @{ count_text(1.0, 0.0) }
        ]
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
        crate::game::GameplayHud { hide_with_inventory: true }
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
                    @{ crafting_section(2) }
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

fn crafting_section(side: u16) -> impl Scene {
    bsn! {
        Node { align_items: AlignItems::Center, column_gap: px(8) }
        Children [
            Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(side, SLOT_SIZE),
                column_gap: px(SLOT_GAP), row_gap: px(SLOT_GAP),
            }
            Children [ { slot_list(CRAFT_START..CRAFT_START + usize::from(side * side), true) } ]
            --
            Text("→")
            TextFont { font_size: FontSize::Px(20.0) }
            TextColor(Color::srgb(0.65,0.65,0.68))
            Pickable::IGNORE
            --
            { slot_list(CRAFT_RESULT..CRAFT_RESULT+1, true) }
        ]
    }
}

pub(crate) fn spawn_inventory_overlay(
    mut commands: Commands,
    assets: Res<crate::player::skin::SkinAssets>,
    access: crate::inventory::session::StationAccess,
    player: Single<
        (
            Option<&crate::inventory::InventorySession>,
            &hollow_features::motion::FixedPosition,
        ),
        With<crate::player::Player>,
    >,
) {
    let (session, position) = *player;
    let Some(layout) = access.layout(session, position.current.as_vec3()) else {
        return;
    };
    let count = layout.containers.iter().flatten().count()
        * crate::block_behavior::containers::CONTAINER_SIZE;
    if count > 0 {
        commands.spawn_scene(container_overlay_scene(count));
    } else if session.is_some() {
        commands.spawn_scene(overlay_scene(
            "Crafting Table",
            bsn! { @{ crafting_section(u16::from(layout.crafting_side)) } },
        ));
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
