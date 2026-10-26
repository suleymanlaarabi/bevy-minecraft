use bevy::{
    feathers::{
        containers::{pane, pane_body, pane_header},
        controls::{
            ButtonVariant, FeathersButton, FeathersRadio, FeathersSlider, FeathersToggleSwitch,
        },
        theme::{InheritableThemeTextColor, ThemeBackgroundColor, ThemedText},
        tokens,
    },
    prelude::*,
    settings::SaveSettingsDeferred,
    ui::Checked,
    ui_widgets::{
        Activate, RadioGroup, ScrollArea, SliderPrecision, SliderStep, SliderValue, ValueChange,
        checkbox_self_update, radio_self_update, slider_self_update,
    },
};

use crate::{game::GameState, voxel::TexturePackId};

use super::model::{FOV_MAX, FOV_MIN, VIEW_DISTANCE_MAX, VIEW_DISTANCE_MIN, normalized_fov};
use super::{AntiAliasingMode, DisplayMode, GraphicsSettings, ShadowQuality};

use super::SAVE_DELAY;

#[derive(Component, Clone, Copy, Debug, Default)]
struct SettingsPanel;

pub(super) fn spawn_settings_page(mut commands: Commands, settings: Res<GraphicsSettings>) {
    commands.spawn_scene(settings_page(*settings));
}

fn settings_page(settings: GraphicsSettings) -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: px(24),
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        GlobalZIndex(200)
        DespawnOnExit::<GameState>(GameState::Settings)
        Children [
            Node {
                width: percent(100),
                max_width: px(780),
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
            }
            Children [
                @settings_panel(settings)
                --
                @back_button()
            ]
        ]
    }
}

/// Shared graphics controls, including persistence and reset, for menus and in-game tools.
pub(crate) fn settings_panel(settings: GraphicsSettings) -> impl Scene {
    let settings = settings.normalized();
    bsn! {
        @pane()
        SettingsPanel
        Node { width: percent(100), max_height: vh(75) }
        Children [
            @pane_header()
            Children [Text("Graphics Settings") ThemedText]
            --
            @pane_body()
            Node { min_height: px(0), overflow: Overflow::scroll_y() }
            ScrollArea
            InheritableThemeTextColor(tokens::TEXT_MAIN)
            Children [
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(8),
                    flex_shrink: 0.0,
                }
                Children [
                    @setting_row("Anti-Aliasing", choice_control(settings.anti_aliasing, [
                        ("Off", AntiAliasingMode::Off),
                        ("FXAA", AntiAliasingMode::Fxaa),
                        ("MSAA 2x", AntiAliasingMode::Msaa2x),
                        ("MSAA 4x", AntiAliasingMode::Msaa4x),
                        ("MSAA 8x", AntiAliasingMode::Msaa8x),
                    ], |settings, value| settings.anti_aliasing = value))
                    --
                    @setting_row("VSync", toggle_control(settings.vsync, |settings, value| settings.vsync = value))
                    --
                    @setting_row("Damage Camera Motion", toggle_control(settings.damage_camera_motion, |settings, value| settings.damage_camera_motion = value))
                    --
                    @setting_row("Survival Sound Volume", graphics_slider(settings.survival_volume, 0.0..=100.0, |settings, value| settings.survival_volume = value))
                    --
                    @setting_row("Occlusion Culling", toggle_control(settings.occlusion_culling, |settings, value| settings.occlusion_culling = value))
                    --
                    @setting_row("Shadow Quality", choice_control(settings.shadow_quality, [
                        ("Off", ShadowQuality::Off),
                        ("Low", ShadowQuality::Low),
                        ("Medium", ShadowQuality::Medium),
                        ("High", ShadowQuality::High),
                    ], |settings, value| settings.shadow_quality = value))
                    --
                    @setting_row("Bloom", toggle_control(settings.bloom, |settings, value| settings.bloom = value))
                    --
                    @setting_row("Clouds", toggle_control(settings.clouds, |settings, value| settings.clouds = value))
                    --
                    @setting_row("Texture Pack", choice_control(settings.texture_pack, [
                        ("Minecraft", TexturePackId::Vanilla),
                        ("Faithful 32x", TexturePackId::Faithful),
                        ("Replete", TexturePackId::Replete),
                        ("Meadow", TexturePackId::Meadow),
                        ("Twilight", TexturePackId::Twilight),
                    ], |settings, value| settings.texture_pack = value))
                    --
                    @setting_row("Field of View", fov_control(settings.field_of_view))
                    --
                    @setting_row("View Distance", view_distance_control(settings.view_distance))
                    --
                    @setting_row("Display Mode", choice_control(settings.display_mode, [
                        ("Windowed", DisplayMode::Windowed),
                        ("Borderless", DisplayMode::BorderlessFullscreen),
                    ], |settings, value| settings.display_mode = value))
                    --
                    @reset_button()
                ]
            ]
        ]
    }
}

fn setting_row(label: &'static str, control: impl Scene) -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            min_height: px(48),
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: px(20),
            row_gap: px(8),
            padding: UiRect::axes(px(10), px(6)),
        }
        Children [
            Node { width: px(170), min_width: px(170), display: Display::Flex, align_items: AlignItems::Center }
            Children [Text(label) ThemedText]
            --
            Node {
                flex_grow: 1.0,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::Center,
            }
            Children [@control]
        ]
    }
}

fn choice_control<T: Component + Copy + Default + PartialEq + Unpin, const N: usize>(
    selected: T,
    choices: [(&'static str, T); N],
    set: fn(&mut GraphicsSettings, T),
) -> impl Scene {
    let choices = choices
        .into_iter()
        .map(|(label, value)| radio_choice(label, value, selected))
        .collect::<Vec<_>>();
    bsn! {
        Node {
            flex_wrap: FlexWrap::Wrap,
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            column_gap: px(12),
            row_gap: px(6),
        }
        RadioGroup
        on(radio_self_update)
        on(move |change: On<ValueChange<Entity>>, choices: Query<&T>, settings: Res<GraphicsSettings>, mut commands: Commands| {
            if let Ok(value) = choices.get(change.value) {
                let mut updated = *settings;
                set(&mut updated, *value);
                store_settings(*settings, updated, &mut commands);
            }
        })
        Children [{choices}]
    }
}

fn radio_choice<T: Component + Copy + Default + PartialEq + Unpin>(
    label: &'static str,
    value: T,
    selected: T,
) -> impl Scene {
    let checked = (value == selected).then_some(bsn! { Checked });
    bsn! {
        @FeathersRadio { @caption: bsn! { Text(label) ThemedText } }
        value
        @checked
    }
}

fn toggle_control(enabled: bool, set: fn(&mut GraphicsSettings, bool)) -> impl Scene {
    let checked = enabled.then_some(bsn! { Checked });
    bsn! {
        @FeathersToggleSwitch
        @checked
        on(checkbox_self_update)
        on(move |change: On<ValueChange<bool>>, settings: Res<GraphicsSettings>, mut commands: Commands| {
            let mut updated = *settings;
            set(&mut updated, change.value);
            store_settings(*settings, updated, &mut commands);
        })
    }
}

fn fov_control(value: f32) -> impl Scene {
    graphics_slider(value, FOV_MIN..=FOV_MAX, |settings, value| {
        settings.field_of_view = normalized_fov(value.round());
    })
}

fn view_distance_control(value: u32) -> impl Scene {
    graphics_slider(
        value as f32,
        VIEW_DISTANCE_MIN as f32..=VIEW_DISTANCE_MAX as f32,
        |settings, value| {
            settings.view_distance =
                (value.round() as u32).clamp(VIEW_DISTANCE_MIN, VIEW_DISTANCE_MAX);
        },
    )
}

fn graphics_slider(
    value: f32,
    range: core::ops::RangeInclusive<f32>,
    set: fn(&mut GraphicsSettings, f32),
) -> impl Scene {
    bsn! {
        @FeathersSlider {
            @min: {*range.start()},
            @max: {*range.end()},
        }
        Node { width: px(320), max_width: percent(100) }
        SliderValue(value)
        SliderStep(1.0)
        SliderPrecision(0)
        on(slider_self_update)
        on(move |change: On<ValueChange<f32>>, settings: Res<GraphicsSettings>, mut commands: Commands| {
            let mut updated = *settings;
            set(&mut updated, change.value);
            store_settings(*settings, updated, &mut commands);
        })
    }
}

fn reset_button() -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn! { Text("Reset Defaults") ThemedText }
        }
        on(reset_settings)
    }
}

fn back_button() -> impl Scene {
    bsn! {
        @FeathersButton {
            @caption: bsn! { Text("Back") ThemedText },
            @variant: ButtonVariant::Primary,
        }
        on(back_to_menu_on_activate)
    }
}

fn reset_settings(
    _activate: On<Activate>,
    panels: Query<Entity, With<SettingsPanel>>,
    settings: Res<GraphicsSettings>,
    mut commands: Commands,
) {
    let defaults = GraphicsSettings::default();
    store_settings(*settings, defaults, &mut commands);
    for panel in &panels {
        commands
            .entity(panel)
            .despawn_related::<Children>()
            .apply_scene(settings_panel(defaults));
    }
}

fn store_settings(current: GraphicsSettings, updated: GraphicsSettings, commands: &mut Commands) {
    let updated = updated.normalized();
    if updated != current {
        commands.insert_resource_if_neq(updated);
        commands.queue(SaveSettingsDeferred(SAVE_DELAY));
    }
}

fn back_to_menu_on_activate(_activate: On<Activate>, mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::Menu);
}
