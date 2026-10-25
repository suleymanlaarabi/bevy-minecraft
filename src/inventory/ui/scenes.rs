use super::interactions::{
    slot_click, slot_drag_end, slot_drag_enter, slot_drag_over, slot_drag_start, slot_out,
    slot_over,
};
use super::{
    CursorStackCount, CursorStackView, HotbarHud, ICON_SIZE, InventoryOverlay, InventorySlotView,
    PANEL_BG, SLOT_BG, SLOT_BORDER, SLOT_GAP, SLOT_SIZE, SlotCount, SlotIcon,
};
use crate::{
    game::GameState,
    inventory::{HOTBAR_SIZE, INVENTORY_SIZE, InventoryState},
};
use bevy::{picking::prelude::Pickable, prelude::*};

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

#[cfg(test)]
mod tests {
    use super::*;
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
