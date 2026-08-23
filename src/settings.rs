use core::time::Duration;

use bevy::{
    anti_alias::fxaa::Fxaa,
    light::DirectionalLightShadowMap,
    prelude::*,
    settings::{
        ReflectSettingsGroup, SaveSettingsDeferred, SaveSettingsSync, SettingsGroup, SettingsPlugin,
    },
    ui_widgets::{
        Slider, SliderPrecision, SliderRange, SliderStep, SliderThumb, SliderValue, ValueChange,
    },
    window::{MonitorSelection, PresentMode, WindowCloseRequested, WindowMode},
};

use crate::{game::GameState, player::PlayerCamera};

const SETTINGS_APP_NAME: &str = "org.hollow.game";
const FOV_MIN: f32 = 60.0;
const FOV_MAX: f32 = 110.0;
const WINDOWED_WIDTH: f32 = 1600.0;
const WINDOWED_HEIGHT: f32 = 900.0;

const PAGE_BACKGROUND: Color = Color::srgb(0.055, 0.055, 0.07);
const PANEL_BACKGROUND: Color = Color::srgb(0.085, 0.085, 0.105);
const ROW_BACKGROUND: Color = Color::srgb(0.115, 0.115, 0.14);
const BUTTON_BACKGROUND: Color = Color::srgb(0.16, 0.16, 0.19);
const BUTTON_HOVERED: Color = Color::srgb(0.23, 0.23, 0.28);
const BUTTON_PRESSED: Color = Color::srgb(0.33, 0.33, 0.4);
const BUTTON_SELECTED: Color = Color::srgb(0.18, 0.42, 0.68);
const BUTTON_SELECTED_HOVERED: Color = Color::srgb(0.23, 0.5, 0.78);
const ACCENT: Color = Color::srgb(0.27, 0.58, 0.9);
const MUTED_TEXT: Color = Color::srgb(0.68, 0.7, 0.76);

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
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

    const fn uses_fxaa(self) -> bool {
        matches!(self, Self::Fxaa)
    }
}

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
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

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    Windowed,
    #[default]
    BorderlessFullscreen,
}

#[derive(Resource, SettingsGroup, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "graphics")]
pub struct GraphicsSettings {
    pub anti_aliasing: AntiAliasingMode,
    pub vsync: bool,
    pub shadow_quality: ShadowQuality,
    pub field_of_view: f32,
    pub display_mode: DisplayMode,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            anti_aliasing: AntiAliasingMode::Msaa4x,
            vsync: true,
            shadow_quality: ShadowQuality::Medium,
            field_of_view: 85.0,
            display_mode: DisplayMode::BorderlessFullscreen,
        }
    }
}

pub struct GameSettingsPlugin;

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SettingsSystem {
    Input,
    Apply,
    Present,
}

impl Plugin for GameSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<AntiAliasingMode>()
            .register_type::<ShadowQuality>()
            .register_type::<DisplayMode>()
            .register_type::<GraphicsSettings>()
            .add_plugins(SettingsPlugin::new(SETTINGS_APP_NAME));

        apply_loaded_window_settings(app);

        app.configure_sets(
            Update,
            (
                SettingsSystem::Input,
                SettingsSystem::Apply,
                SettingsSystem::Present,
            )
                .chain(),
        )
        .add_observer(handle_fov_change)
        .add_systems(OnEnter(GameState::Settings), spawn_settings_page)
        .add_systems(
            Update,
            (
                sanitize_graphics_settings,
                handle_settings_buttons,
                handle_settings_escape,
            )
                .chain()
                .in_set(SettingsSystem::Input)
                .run_if(in_state(GameState::Settings)),
        )
        .add_systems(
            Update,
            apply_changed_graphics_settings
                .in_set(SettingsSystem::Apply)
                .run_if(resource_changed::<GraphicsSettings>),
        )
        .add_systems(
            Update,
            apply_graphics_to_new_targets.in_set(SettingsSystem::Apply),
        )
        .add_systems(
            Update,
            sync_settings_ui
                .in_set(SettingsSystem::Present)
                .run_if(in_state(GameState::Settings)),
        )
        .add_systems(Update, handle_window_close);
    }
}

fn apply_loaded_window_settings(app: &mut App) {
    let Some(settings) = app.world().get_resource::<GraphicsSettings>().copied() else {
        return;
    };
    let world = app.world_mut();
    let mut windows = world.query::<&mut Window>();
    for mut window in windows.iter_mut(world) {
        apply_window_settings(&settings, &mut window);
    }
}

#[derive(Component, Clone, Copy, Debug, Default)]
enum SettingsAction {
    SetAntiAliasing(AntiAliasingMode),
    ToggleVsync,
    SetShadowQuality(ShadowQuality),
    SetDisplayMode(DisplayMode),
    Reset,
    #[default]
    Back,
}

#[derive(Component, Clone, Copy, Debug, Default)]
struct SettingsControl {
    action: SettingsAction,
}

#[derive(Component, Clone, Copy, Debug, Default)]
struct VsyncValueText;

#[derive(Component, Clone, Copy, Debug, Default)]
struct FovSlider;

#[derive(Component, Clone, Copy, Debug, Default)]
struct FovSliderThumb;

#[derive(Component, Clone, Copy, Debug, Default)]
struct FovValueText;

fn spawn_settings_page(mut commands: Commands, settings: Res<GraphicsSettings>) {
    commands.spawn_scene_list(settings_page(*settings));
}

fn settings_page(settings: GraphicsSettings) -> impl SceneList {
    bsn_list! [
        Node {
            width: percent(100),
            height: percent(100),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: px(30),
        }
        BackgroundColor(PAGE_BACKGROUND)
        GlobalZIndex(200)
        DespawnOnExit::<GameState>(GameState::Settings)
        Children [
            Node {
                width: percent(100),
                max_width: px(1080),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                row_gap: px(18),
                padding: px(28),
                border: px(1),
                border_radius: BorderRadius::all(px(12)),
            }
            BackgroundColor(PANEL_BACKGROUND)
            BorderColor::all(Color::srgb(0.22, 0.22, 0.27))
            Children [
                Node {
                    width: percent(100),
                    display: Display::Flex,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(4),
                }
                Children [
                    Text::new("Graphics Settings")
                    TextFont { font_size: px(42) }
                    TextColor(Color::WHITE),
                    Text::new("Changes are applied and saved automatically")
                    TextFont { font_size: px(18) }
                    TextColor(MUTED_TEXT)
                ],

                setting_row("Anti-Aliasing", bsn_list![
                    option_button("Off", SettingsAction::SetAntiAliasing(AntiAliasingMode::Off)),
                    option_button("FXAA", SettingsAction::SetAntiAliasing(AntiAliasingMode::Fxaa)),
                    option_button("MSAA 2x", SettingsAction::SetAntiAliasing(AntiAliasingMode::Msaa2x)),
                    option_button("MSAA 4x", SettingsAction::SetAntiAliasing(AntiAliasingMode::Msaa4x)),
                    option_button("MSAA 8x", SettingsAction::SetAntiAliasing(AntiAliasingMode::Msaa8x)),
                ]),

                setting_row("VSync", bsn_list![
                    vsync_button(settings.vsync)
                ]),

                setting_row("Shadow Quality", bsn_list![
                    option_button("Off", SettingsAction::SetShadowQuality(ShadowQuality::Off)),
                    option_button("Low", SettingsAction::SetShadowQuality(ShadowQuality::Low)),
                    option_button("Medium", SettingsAction::SetShadowQuality(ShadowQuality::Medium)),
                    option_button("High", SettingsAction::SetShadowQuality(ShadowQuality::High)),
                ]),

                setting_row("Field of View", bsn_list![
                    fov_control(settings.field_of_view)
                ]),

                setting_row("Display Mode", bsn_list![
                    option_button("Windowed", SettingsAction::SetDisplayMode(DisplayMode::Windowed)),
                    option_button("Borderless", SettingsAction::SetDisplayMode(DisplayMode::BorderlessFullscreen)),
                ]),

                Node {
                    width: percent(100),
                    display: Display::Flex,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    column_gap: px(12),
                    margin: UiRect::top(px(4)),
                }
                Children [
                    footer_button("Reset Defaults", SettingsAction::Reset),
                    footer_button("Back", SettingsAction::Back)
                ]
            ]
        ]
    ]
}

fn setting_row(label: &'static str, controls: impl SceneList) -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            min_height: px(76),
            display: Display::Flex,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: px(20),
            row_gap: px(10),
            padding: UiRect::axes(px(18), px(14)),
            border_radius: BorderRadius::all(px(8)),
        }
        BackgroundColor(ROW_BACKGROUND)
        Children [
            Node {
                width: px(190),
                min_width: px(190),
                align_items: AlignItems::Center
            }
            Children [
                Text::new(label)
                TextFont { font_size: px(21) }
                TextColor(Color::WHITE)
            ],
            Node {
                display: Display::Flex,
                flex_grow: 1.0,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::Center,
                column_gap: px(8),
                row_gap: px(8),
            }
            Children [ {controls} ]
        ]
    }
}

fn option_button(label: &'static str, action: SettingsAction) -> impl Scene {
    bsn! {
        Button
        Interaction::default()
        SettingsControl { action }
        Node {
            min_width: px(100),
            height: px(42),
            display: Display::Flex,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::horizontal(px(14)),
            border_radius: BorderRadius::all(px(7)),
        }
        BackgroundColor(BUTTON_BACKGROUND)
        Children [
            Text::new(label)
            TextFont { font_size: px(17) }
            TextColor(Color::WHITE)
        ]
    }
}

fn vsync_button(enabled: bool) -> impl Scene {
    let label = if enabled { "On" } else { "Off" };
    bsn! {
        Button
        Interaction::default()
        SettingsControl { action: SettingsAction::ToggleVsync }
        Node {
            min_width: px(110),
            height: px(42),
            display: Display::Flex,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::horizontal(px(14)),
            border_radius: BorderRadius::all(px(7)),
        }
        BackgroundColor(BUTTON_BACKGROUND)
        Children [
            Text::new(label)
            VsyncValueText
            TextFont { font_size: px(17) }
            TextColor(Color::WHITE)
        ]
    }
}

fn footer_button(label: &'static str, action: SettingsAction) -> impl Scene {
    bsn! {
        Button
        Interaction::default()
        SettingsControl { action }
        Node {
            min_width: px(170),
            height: px(50),
            display: Display::Flex,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            padding: UiRect::horizontal(px(20)),
            border_radius: BorderRadius::all(px(8)),
        }
        BackgroundColor(BUTTON_BACKGROUND)
        Children [
            Text::new(label)
            TextFont { font_size: px(19) }
            TextColor(Color::WHITE)
        ]
    }
}

fn fov_control(value: f32) -> impl Scene {
    bsn! {
        Node {
            min_width: px(430),
            height: px(42),
            display: Display::Flex,
            align_items: AlignItems::Center,
            column_gap: px(16),
        }
        Children [
            Node {
                position_type: PositionType::Relative,
                width: percent(100),
                height: px(20),
                display: Display::Flex,
                align_items: AlignItems::Center,
            }
            FovSlider
            Slider::default()
            SliderValue(value)
            SliderRange::new(FOV_MIN, FOV_MAX)
            SliderStep(1.0)
            SliderPrecision(0)
            Children [
                Node {
                    width: percent(100),
                    height: px(6),
                    border_radius: BorderRadius::all(px(3)),
                }
                BackgroundColor(Color::srgb(0.055, 0.055, 0.07)),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    right: px(16),
                    top: px(0),
                    bottom: px(0),
                }
                Children [
                    Node {
                        position_type: PositionType::Absolute,
                        left: percent(0),
                        width: px(16),
                        height: px(16),
                        border_radius: BorderRadius::MAX,
                    }
                    FovSliderThumb
                    SliderThumb
                    BackgroundColor(ACCENT)
                ]
            ],
            Node {
                width: px(62),
                display: Display::Flex,
                justify_content: JustifyContent::FlexEnd,
            }
            Children [
                Text::new(format!("{}", value.round() as i32))
                FovValueText
                TextFont { font_size: px(18) }
                TextColor(Color::WHITE)
            ]
        ]
    }
}

fn handle_settings_buttons(
    mut commands: Commands,
    mut settings: ResMut<GraphicsSettings>,
    mut next_state: ResMut<NextState<GameState>>,
    controls: Query<(&Interaction, &SettingsControl), Changed<Interaction>>,
) {
    let mut updated = *settings;

    for (interaction, control) in &controls {
        if *interaction != Interaction::Pressed {
            continue;
        }

        match control.action {
            SettingsAction::SetAntiAliasing(value) => updated.anti_aliasing = value,
            SettingsAction::ToggleVsync => updated.vsync = !updated.vsync,
            SettingsAction::SetShadowQuality(value) => updated.shadow_quality = value,
            SettingsAction::SetDisplayMode(value) => updated.display_mode = value,
            SettingsAction::Reset => updated = GraphicsSettings::default(),
            SettingsAction::Back => next_state.set(GameState::Menu),
        }
    }

    if updated != *settings {
        *settings = updated;
        queue_settings_save(&mut commands);
    }
}

fn sanitize_graphics_settings(mut settings: ResMut<GraphicsSettings>, mut commands: Commands) {
    let field_of_view = normalized_fov(settings.field_of_view);
    if settings.field_of_view != field_of_view {
        settings.field_of_view = field_of_view;
        queue_settings_save(&mut commands);
    }
}

fn normalized_fov(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(FOV_MIN, FOV_MAX)
    } else {
        GraphicsSettings::default().field_of_view
    }
}

fn handle_fov_change(
    event: On<ValueChange<f32>>,
    sliders: Query<(), With<FovSlider>>,
    mut settings: ResMut<GraphicsSettings>,
    mut commands: Commands,
) {
    if sliders.get(event.source).is_err() {
        return;
    }

    let value = normalized_fov(event.value.round());
    commands.entity(event.source).insert(SliderValue(value));
    if settings.field_of_view != value {
        settings.field_of_view = value;
        queue_settings_save(&mut commands);
    }
}

fn handle_settings_escape(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        next_state.set(GameState::Menu);
    }
}

fn sync_settings_ui(
    settings: Res<GraphicsSettings>,
    mut controls: Query<(&Interaction, &SettingsControl, &mut BackgroundColor)>,
    mut vsync_text: Query<&mut Text, With<VsyncValueText>>,
    mut fov_text: Query<&mut Text, (With<FovValueText>, Without<VsyncValueText>)>,
    sliders: Query<(Entity, &SliderValue), With<FovSlider>>,
    mut thumbs: Query<&mut Node, With<FovSliderThumb>>,
    mut commands: Commands,
) {
    for (interaction, control, mut background) in &mut controls {
        let selected = control.action.is_selected(&settings);
        background.0 = match (*interaction, selected) {
            (Interaction::Pressed, _) => BUTTON_PRESSED,
            (Interaction::Hovered, true) => BUTTON_SELECTED_HOVERED,
            (Interaction::Hovered, false) => BUTTON_HOVERED,
            (Interaction::None, true) => BUTTON_SELECTED,
            (Interaction::None, false) => BUTTON_BACKGROUND,
        };
    }

    for mut text in &mut vsync_text {
        text.0 = if settings.vsync { "On" } else { "Off" }.into();
    }
    for mut text in &mut fov_text {
        text.0 = format!("{}", settings.field_of_view.round() as i32);
    }

    for (entity, value) in &sliders {
        if value.0 != settings.field_of_view {
            commands
                .entity(entity)
                .insert(SliderValue(settings.field_of_view));
        }
    }

    let position = ((settings.field_of_view - FOV_MIN) / (FOV_MAX - FOV_MIN)) * 100.0;
    for mut thumb in &mut thumbs {
        thumb.left = percent(position);
    }
}

impl SettingsAction {
    fn is_selected(self, settings: &GraphicsSettings) -> bool {
        match self {
            Self::SetAntiAliasing(value) => settings.anti_aliasing == value,
            Self::ToggleVsync => settings.vsync,
            Self::SetShadowQuality(value) => settings.shadow_quality == value,
            Self::SetDisplayMode(value) => settings.display_mode == value,
            Self::Reset | Self::Back => false,
        }
    }
}

fn queue_settings_save(commands: &mut Commands) {
    commands.queue(SaveSettingsDeferred(Duration::from_millis(250)));
}

fn apply_changed_graphics_settings(
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window>,
    mut cameras: Query<(Entity, &mut Projection), With<PlayerCamera>>,
    mut lights: Query<&mut DirectionalLight>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut commands: Commands,
) {
    for mut window in &mut windows {
        apply_window_settings(&settings, &mut window);
    }
    for (entity, mut projection) in &mut cameras {
        apply_camera_settings(&settings, entity, &mut projection, &mut commands);
    }
    for mut light in &mut lights {
        apply_light_settings(&settings, &mut light);
    }
    apply_shadow_map_settings(&settings, &mut shadow_map);
}

type AddedPlayerCameraQuery<'w, 's> =
    Query<'w, 's, (Entity, &'static mut Projection), (With<PlayerCamera>, Added<PlayerCamera>)>;

fn apply_graphics_to_new_targets(
    settings: Res<GraphicsSettings>,
    mut windows: Query<&mut Window, Added<Window>>,
    mut cameras: AddedPlayerCameraQuery,
    mut lights: Query<&mut DirectionalLight, Added<DirectionalLight>>,
    mut commands: Commands,
) {
    for mut window in &mut windows {
        apply_window_settings(&settings, &mut window);
    }
    for (entity, mut projection) in &mut cameras {
        apply_camera_settings(&settings, entity, &mut projection, &mut commands);
    }
    for mut light in &mut lights {
        apply_light_settings(&settings, &mut light);
    }
}

fn apply_window_settings(settings: &GraphicsSettings, window: &mut Window) {
    let present_mode = if settings.vsync {
        PresentMode::Fifo
    } else {
        PresentMode::AutoNoVsync
    };
    if window.present_mode != present_mode {
        window.present_mode = present_mode;
    }

    match settings.display_mode {
        DisplayMode::Windowed => {
            if window.mode != WindowMode::Windowed {
                window.mode = WindowMode::Windowed;
                window.resolution.set(WINDOWED_WIDTH, WINDOWED_HEIGHT);
            }
        }
        DisplayMode::BorderlessFullscreen => {
            let mode = WindowMode::BorderlessFullscreen(MonitorSelection::Primary);
            if window.mode != mode {
                window.mode = mode;
            }
        }
    }
}

fn apply_camera_settings(
    settings: &GraphicsSettings,
    entity: Entity,
    projection: &mut Projection,
    commands: &mut Commands,
) {
    apply_projection_settings(settings, projection);

    let mut camera = commands.entity(entity);
    camera.insert(settings.anti_aliasing.msaa());
    if settings.anti_aliasing.uses_fxaa() {
        camera.insert(Fxaa::default());
    } else {
        camera.remove::<Fxaa>();
    }
}

fn apply_projection_settings(settings: &GraphicsSettings, projection: &mut Projection) {
    if let Projection::Perspective(perspective) = projection {
        perspective.fov = normalized_fov(settings.field_of_view).to_radians();
    }
}

fn apply_light_settings(settings: &GraphicsSettings, light: &mut DirectionalLight) {
    light.shadow_maps_enabled = settings.shadow_quality != ShadowQuality::Off;
}

fn apply_shadow_map_settings(
    settings: &GraphicsSettings,
    shadow_map: &mut DirectionalLightShadowMap,
) {
    if let Some(size) = settings.shadow_quality.map_size()
        && shadow_map.size != size
    {
        shadow_map.size = size;
    }
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
