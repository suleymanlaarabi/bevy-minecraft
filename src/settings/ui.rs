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
        Activate, RadioGroup, SliderPrecision, SliderStep, SliderValue, ValueChange,
        checkbox_self_update, radio_self_update, slider_self_update,
    },
};

use crate::{game::GameState, voxel::TexturePackId};

use super::model::{FOV_MAX, FOV_MIN, VIEW_DISTANCE_MAX, VIEW_DISTANCE_MIN, normalized_fov};
use super::{AntiAliasingMode, DisplayMode, GraphicsSettings, ShadowQuality};

use super::SAVE_DELAY;

#[derive(Component, Clone, Copy, Debug, Default)]
struct SettingsPage;

trait GraphicsChoice: Component + Clone + Copy + Default + PartialEq + Unpin {
    fn set(self, settings: &mut GraphicsSettings);
}

impl GraphicsChoice for AntiAliasingMode {
    fn set(self, settings: &mut GraphicsSettings) {
        settings.anti_aliasing = self;
    }
}

impl GraphicsChoice for ShadowQuality {
    fn set(self, settings: &mut GraphicsSettings) {
        settings.shadow_quality = self;
    }
}

impl GraphicsChoice for TexturePackId {
    fn set(self, settings: &mut GraphicsSettings) {
        settings.texture_pack = self;
    }
}

impl GraphicsChoice for DisplayMode {
    fn set(self, settings: &mut GraphicsSettings) {
        settings.display_mode = self;
    }
}

pub(super) fn spawn_settings_page(mut commands: Commands, settings: Res<GraphicsSettings>) {
    commands.spawn_scene(settings_page(settings.normalized()));
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
        SettingsPage
        ThemeBackgroundColor(tokens::WINDOW_BG)
        GlobalZIndex(200)
        DespawnOnExit::<GameState>(GameState::Settings)
        Children [
            @pane()
            Node {
                width: percent(100),
                max_width: px(780),
            }
            Children [
                @pane_header()
                Children [Text("Graphics Settings") ThemedText]
                --
                @pane_body()
                Node { row_gap: px(8) }
                InheritableThemeTextColor(tokens::TEXT_MAIN)
                Children [
                    @setting_row("Anti-Aliasing", anti_aliasing_control(settings.anti_aliasing))
                    --
                    @setting_row("VSync", vsync_control(settings.vsync))
                    --
                    @setting_row("Occlusion Culling", occlusion_culling_control(settings.occlusion_culling))
                    --
                    @setting_row("Shadow Quality", shadow_quality_control(settings.shadow_quality))
                    --
                    @setting_row("Texture Pack", texture_pack_control(settings.texture_pack))
                    --
                    @setting_row("Field of View", fov_control(settings.field_of_view))
                    --
                    @setting_row("View Distance", view_distance_control(settings.view_distance))
                    --
                    @setting_row("Display Mode", display_mode_control(settings.display_mode))
                    --
                    Node {
                        width: percent(100),
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: px(8),
                        margin: UiRect::top(px(4)),
                    }
                    Children [
                        @reset_button()
                        --
                        @back_button()
                    ]
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

fn radio_group() -> impl Scene {
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
    }
}

fn anti_aliasing_control(selected: AntiAliasingMode) -> impl Scene {
    bsn! {
        @radio_group()
        on(set_choice::<AntiAliasingMode>)
        Children [
            @radio_choice("Off", AntiAliasingMode::Off, selected)
            --
            @radio_choice("FXAA", AntiAliasingMode::Fxaa, selected)
            --
            @radio_choice("MSAA 2x", AntiAliasingMode::Msaa2x, selected)
            --
            @radio_choice("MSAA 4x", AntiAliasingMode::Msaa4x, selected)
            --
            @radio_choice("MSAA 8x", AntiAliasingMode::Msaa8x, selected)
        ]
    }
}

fn radio_choice<T: GraphicsChoice>(label: &'static str, value: T, selected: T) -> impl Scene {
    let checked = (value == selected).then_some(bsn! { Checked });
    bsn! {
        @FeathersRadio { @caption: bsn! { Text(label) ThemedText } }
        value
        @checked
    }
}

fn shadow_quality_control(selected: ShadowQuality) -> impl Scene {
    bsn! {
        @radio_group()
        on(set_choice::<ShadowQuality>)
        Children [
            @radio_choice("Off", ShadowQuality::Off, selected)
            --
            @radio_choice("Low", ShadowQuality::Low, selected)
            --
            @radio_choice("Medium", ShadowQuality::Medium, selected)
            --
            @radio_choice("High", ShadowQuality::High, selected)
        ]
    }
}

fn texture_pack_control(selected: TexturePackId) -> impl Scene {
    bsn! {
        @radio_group()
        on(set_choice::<TexturePackId>)
        Children [
            @radio_choice("Minecraft", TexturePackId::Vanilla, selected)
            --
            @radio_choice("Faithful 32x", TexturePackId::Faithful, selected)
            --
            @radio_choice("Replete", TexturePackId::Replete, selected)
            --
            @radio_choice("Meadow", TexturePackId::Meadow, selected)
            --
            @radio_choice("Twilight", TexturePackId::Twilight, selected)
        ]
    }
}

fn display_mode_control(selected: DisplayMode) -> impl Scene {
    bsn! {
        @radio_group()
        on(set_choice::<DisplayMode>)
        Children [
            @radio_choice("Windowed", DisplayMode::Windowed, selected)
            --
            @radio_choice("Borderless", DisplayMode::BorderlessFullscreen, selected)
        ]
    }
}

fn vsync_control(enabled: bool) -> impl Scene {
    let checked = enabled.then_some(bsn! { Checked });
    bsn! {
        @FeathersToggleSwitch
        @checked
        on(checkbox_self_update)
        on(set_vsync)
    }
}

fn fov_control(value: f32) -> impl Scene {
    bsn! {
        @FeathersSlider {
            @min: FOV_MIN,
            @max: FOV_MAX,
        }
        Node { width: px(320) }
        SliderValue(value)
        SliderStep(1.0)
        SliderPrecision(0)
        on(slider_self_update)
        on(set_field_of_view)
    }
}

fn view_distance_control(value: u32) -> impl Scene {
    bsn! {
        @FeathersSlider {
            @min: {VIEW_DISTANCE_MIN as f32},
            @max: {VIEW_DISTANCE_MAX as f32},
        }
        Node { width: px(320) }
        SliderValue({value as f32})
        SliderStep(1.0)
        SliderPrecision(0)
        on(slider_self_update)
        on(set_view_distance)
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

fn set_choice<T: GraphicsChoice>(
    change: On<ValueChange<Entity>>,
    choices: Query<&T>,
    settings: Res<GraphicsSettings>,
    mut commands: Commands,
) {
    if let Ok(choice) = choices.get(change.value) {
        let mut updated = *settings;
        choice.set(&mut updated);
        store_settings(*settings, updated, &mut commands);
    }
}

fn occlusion_culling_control(enabled: bool) -> impl Scene {
    let checked = enabled.then_some(bsn! { Checked });
    bsn! {
        @FeathersToggleSwitch
        @checked
        on(checkbox_self_update)
        on(set_occlusion_culling)
    }
}

fn set_occlusion_culling(
    change: On<ValueChange<bool>>,
    settings: Res<GraphicsSettings>,
    mut commands: Commands,
) {
    let mut updated = *settings;
    updated.occlusion_culling = change.value;
    store_settings(*settings, updated, &mut commands);
}

fn set_vsync(
    change: On<ValueChange<bool>>,
    settings: Res<GraphicsSettings>,
    mut commands: Commands,
) {
    let mut updated = *settings;
    updated.vsync = change.value;
    store_settings(*settings, updated, &mut commands);
}

fn set_field_of_view(
    change: On<ValueChange<f32>>,
    settings: Res<GraphicsSettings>,
    mut commands: Commands,
) {
    let mut updated = *settings;
    updated.field_of_view = normalized_fov(change.value.round());
    store_settings(*settings, updated, &mut commands);
}

fn set_view_distance(
    change: On<ValueChange<f32>>,
    settings: Res<GraphicsSettings>,
    mut commands: Commands,
) {
    let mut updated = *settings;
    updated.view_distance =
        (change.value.round() as u32).clamp(VIEW_DISTANCE_MIN, VIEW_DISTANCE_MAX);
    store_settings(*settings, updated, &mut commands);
}

fn reset_settings(
    _activate: On<Activate>,
    pages: Query<Entity, With<SettingsPage>>,
    settings: Res<GraphicsSettings>,
    mut commands: Commands,
) {
    let defaults = GraphicsSettings::default();
    store_settings(*settings, defaults, &mut commands);
    for page in &pages {
        commands.entity(page).despawn();
    }
    commands.spawn_scene(settings_page(defaults));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sliders_preserve_the_configured_initial_values() {
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            AssetPlugin::default(),
            bevy::scene::ScenePlugin,
        ));
        app.init_asset::<Font>();
        let world = app.world_mut();
        let fov = world.spawn_scene(fov_control(97.0)).unwrap().id();
        let distance = world.spawn_scene(view_distance_control(17)).unwrap().id();
        assert_eq!(world.get::<SliderValue>(fov).unwrap().0, 97.0);
        assert_eq!(world.get::<SliderValue>(distance).unwrap().0, 17.0);
    }
}
