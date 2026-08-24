use core::time::Duration;

use bevy::{
    feathers::{
        containers::{pane, pane_body, pane_header},
        controls::{
            ButtonVariant, FeathersButton, FeathersRadio, FeathersSlider, FeathersToggleSwitch,
        },
        theme::{InheritableThemeTextColor, ThemeBackgroundColor, ThemedText},
        tokens,
    },
    input::common_conditions::input_just_pressed,
    prelude::*,
    settings::{
        ReflectSettingsGroup, SaveSettingsDeferred, SaveSettingsSync, SettingsGroup, SettingsPlugin,
    },
    ui::Checked,
    ui_widgets::{
        Activate, RadioGroup, SliderPrecision, SliderStep, ValueChange, checkbox_self_update,
        radio_self_update, slider_self_update,
    },
    window::WindowCloseRequested,
};

use crate::game::GameState;

mod apply;

const SETTINGS_APP_NAME: &str = "org.hollow.game";
const FOV_MIN: f32 = 60.0;
const FOV_MAX: f32 = 110.0;
const VIEW_DISTANCE_MIN: u32 = 4;
const VIEW_DISTANCE_MAX: u32 = 32;
const SAVE_DELAY: Duration = Duration::from_millis(250);

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AntiAliasingMode {
    Off,
    Fxaa,
    Msaa2x,
    #[default]
    Msaa4x,
    Msaa8x,
}

impl AntiAliasingMode {
    const fn msaa(self) -> Msaa {
        match self {
            Self::Off | Self::Fxaa => Msaa::Off,
            Self::Msaa2x => Msaa::Sample2,
            Self::Msaa4x => Msaa::Sample4,
            Self::Msaa8x => Msaa::Sample8,
        }
    }
}

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadowQuality {
    Off,
    Low,
    #[default]
    Medium,
    High,
}

impl ShadowQuality {
    const fn map_size(self) -> Option<usize> {
        match self {
            Self::Off => None,
            Self::Low => Some(1024),
            Self::Medium => Some(2048),
            Self::High => Some(4096),
        }
    }
}

#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    Windowed,
    #[default]
    BorderlessFullscreen,
}

#[derive(Resource, SettingsGroup, Reflect, Clone, Copy, Debug, PartialEq)]
#[component(immutable)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "graphics")]
pub struct GraphicsSettings {
    pub anti_aliasing: AntiAliasingMode,
    pub vsync: bool,
    pub shadow_quality: ShadowQuality,
    pub field_of_view: f32,
    pub view_distance: u32,
    pub display_mode: DisplayMode,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            anti_aliasing: AntiAliasingMode::Msaa4x,
            vsync: true,
            shadow_quality: ShadowQuality::Medium,
            field_of_view: 85.0,
            view_distance: 10,
            display_mode: DisplayMode::BorderlessFullscreen,
        }
    }
}

impl GraphicsSettings {
    fn normalized(mut self) -> Self {
        self.field_of_view = normalized_fov(self.field_of_view);
        self.view_distance = self.effective_view_distance();
        self
    }

    pub(crate) fn effective_view_distance(&self) -> u32 {
        self.view_distance
            .clamp(VIEW_DISTANCE_MIN, VIEW_DISTANCE_MAX)
    }
}

pub struct GameSettingsPlugin;

impl Plugin for GameSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<AntiAliasingMode>()
            .register_type::<ShadowQuality>()
            .register_type::<DisplayMode>()
            .register_type::<GraphicsSettings>();
        apply::register(app);
        app.add_plugins(SettingsPlugin::new(SETTINGS_APP_NAME))
            .add_systems(OnEnter(GameState::Settings), spawn_settings_page)
            .add_systems(
                Update,
                back_to_menu
                    .run_if(in_state(GameState::Settings))
                    .run_if(input_just_pressed(KeyCode::Escape)),
            )
            .add_systems(Update, handle_window_close);
    }
}

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

impl GraphicsChoice for DisplayMode {
    fn set(self, settings: &mut GraphicsSettings) {
        settings.display_mode = self;
    }
}

fn spawn_settings_page(mut commands: Commands, settings: Res<GraphicsSettings>) {
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
        Children [(
            pane()
            Node {
                width: percent(100),
                max_width: px(780),
            }
            Children [
                pane_header()
                Children [(Text("Graphics Settings") ThemedText)],
                pane_body()
                Node { row_gap: px(8) }
                InheritableThemeTextColor(tokens::TEXT_MAIN)
                Children [
                    setting_row("Anti-Aliasing", anti_aliasing_control(settings.anti_aliasing)),
                    setting_row("VSync", vsync_control(settings.vsync)),
                    setting_row("Shadow Quality", shadow_quality_control(settings.shadow_quality)),
                    setting_row("Field of View", fov_control(settings.field_of_view)),
                    setting_row("View Distance", view_distance_control(settings.view_distance)),
                    setting_row("Display Mode", display_mode_control(settings.display_mode)),
                    Node {
                        width: percent(100),
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: px(8),
                        margin: UiRect::top(px(4)),
                    }
                    Children [
                        reset_button(),
                        back_button(),
                    ]
                ]
            ]
        )]
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
            Children [(Text(label) ThemedText)],
            Node {
                flex_grow: 1.0,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::Center,
            }
            Children [control]
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
        radio_group()
        on(set_choice::<AntiAliasingMode>)
        Children [
            radio_choice("Off", AntiAliasingMode::Off, selected),
            radio_choice("FXAA", AntiAliasingMode::Fxaa, selected),
            radio_choice("MSAA 2x", AntiAliasingMode::Msaa2x, selected),
            radio_choice("MSAA 4x", AntiAliasingMode::Msaa4x, selected),
            radio_choice("MSAA 8x", AntiAliasingMode::Msaa8x, selected),
        ]
    }
}

fn radio_choice<T: GraphicsChoice>(label: &'static str, value: T, selected: T) -> impl Scene {
    let checked = (value == selected).then_some(bsn! { Checked });
    bsn! {
        @FeathersRadio { @caption: bsn! { Text(label) ThemedText } }
        template_value(value)
        checked
    }
}

fn shadow_quality_control(selected: ShadowQuality) -> impl Scene {
    bsn! {
        radio_group()
        on(set_choice::<ShadowQuality>)
        Children [
            radio_choice("Off", ShadowQuality::Off, selected),
            radio_choice("Low", ShadowQuality::Low, selected),
            radio_choice("Medium", ShadowQuality::Medium, selected),
            radio_choice("High", ShadowQuality::High, selected),
        ]
    }
}

fn display_mode_control(selected: DisplayMode) -> impl Scene {
    bsn! {
        radio_group()
        on(set_choice::<DisplayMode>)
        Children [
            radio_choice("Windowed", DisplayMode::Windowed, selected),
            radio_choice("Borderless", DisplayMode::BorderlessFullscreen, selected),
        ]
    }
}

fn vsync_control(enabled: bool) -> impl Scene {
    let checked = enabled.then_some(bsn! { Checked });
    bsn! {
        @FeathersToggleSwitch
        checked
        on(checkbox_self_update)
        on(set_vsync)
    }
}

fn fov_control(value: f32) -> impl Scene {
    bsn! {
        @FeathersSlider {
            @value: value,
            @min: FOV_MIN,
            @max: FOV_MAX,
        }
        Node { width: px(320) }
        SliderStep(1.0)
        SliderPrecision(0)
        on(slider_self_update)
        on(set_field_of_view)
    }
}

fn view_distance_control(value: u32) -> impl Scene {
    bsn! {
        @FeathersSlider {
            @value: {value as f32},
            @min: {VIEW_DISTANCE_MIN as f32},
            @max: {VIEW_DISTANCE_MAX as f32},
        }
        Node { width: px(320) }
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

fn normalized_fov(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(FOV_MIN, FOV_MAX)
    } else {
        GraphicsSettings::default().field_of_view
    }
}

fn back_to_menu(mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::Menu);
}

fn back_to_menu_on_activate(_activate: On<Activate>, mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::Menu);
}

fn handle_window_close(
    mut close_events: MessageReader<WindowCloseRequested>,
    mut commands: Commands,
) {
    if close_events.read().next().is_some() {
        commands.queue(SaveSettingsSync::IfChanged);
        commands.write_message(AppExit::Success);
    }
}
