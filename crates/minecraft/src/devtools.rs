//! Developer panel, toggled with P: world controls and shared graphics settings.
//!
//! While it is open the cursor is free and the player neither moves nor interacts,
//! like with the inventory.

use bevy::{
    ecs::template::{EntityTemplate, OptionTemplate},
    feathers::{
        containers::{pane, pane_body, pane_header},
        controls::{FeathersSlider, FeathersToggleSwitch},
        focus::FocusIndicator,
        theme::{InheritableThemeTextColor, ThemeBackgroundColor, ThemeToken, ThemedText},
        tokens,
    },
    prelude::*,
    ui::Checked,
    ui_widgets::{
        SelectedTab, SliderPrecision, SliderStep, SliderValue, Tab, TabActivation, TabList,
        ValueChange, checkbox_self_update, slider_self_update, tablist_self_update,
    },
};

use crate::{
    character::Flying,
    game::GameState,
    inventory::InventoryState,
    player::{Player, clear_player_input},
    settings::{GraphicsSettings, SimulationSettings, settings_panel},
    sky::TimeOfDay,
};

/// Fastest day cycle offered, in days per normal day length.
const MAX_DAY_SPEED: f32 = 100.0;

#[derive(SubStates, Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[source(GameState = GameState::Game)]
pub enum DevPanelState {
    #[default]
    Closed,
    Open,
}

/// Run condition for gameplay input: the developer panel is not open.
pub(crate) fn panel_closed(state: Option<Res<State<DevPanelState>>>) -> bool {
    state.is_none_or(|state| *state.get() == DevPanelState::Closed)
}

#[derive(Component, Clone, Copy, Debug, Default)]
struct TimeSlider;

#[derive(Component, Clone, Copy, Debug, Default)]
struct ClockText;

#[derive(Component, Clone, Copy, Debug, Default)]
#[require(hollow_features::cursor::FreeCursor)]
struct DevPanel;

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PanelTab {
    #[default]
    World,
    Settings,
}

pub struct DevToolsPlugin;

impl Plugin for DevToolsPlugin {
    fn build(&self, app: &mut App) {
        app.add_sub_state::<DevPanelState>()
            .add_systems(
                Update,
                toggle_panel
                    .run_if(crate::world::playing)
                    .run_if(in_state(InventoryState::Closed))
                    .run_if(any_with_component::<Player>),
            )
            // Keys held when the panel opens must not keep the player moving.
            .add_systems(
                OnEnter(DevPanelState::Open),
                (clear_player_input, spawn_panel),
            )
            .add_systems(
                Update,
                sync_clock
                    .after(crate::sky::SkySystems::Advance)
                    .run_if(in_state(DevPanelState::Open)),
            );
    }
}

fn toggle_panel(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<DevPanelState>>,
    mut next_state: ResMut<NextState<DevPanelState>>,
) {
    if keyboard.just_pressed(KeyCode::KeyP) {
        next_state.set(match state.get() {
            DevPanelState::Closed => DevPanelState::Open,
            DevPanelState::Open => DevPanelState::Closed,
        });
    }
}

fn spawn_panel(
    mut commands: Commands,
    time: Res<TimeOfDay>,
    settings: Res<GraphicsSettings>,
    simulation: Res<SimulationSettings>,
    flying: Query<(), (With<Player>, With<Flying>)>,
) {
    commands.spawn_scene(panel(!flying.is_empty(), *time, *settings, *simulation));
}

fn panel(
    flying: bool,
    time: TimeOfDay,
    settings: GraphicsSettings,
    simulation: SimulationSettings,
) -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            right: px(12),
            width: px(340),
            max_width: percent(95),
            max_height: vh(95),
        }
        DevPanel
        GlobalZIndex(300)
        DespawnOnExit::<DevPanelState>(DevPanelState::Open)
        Children [
            @pane()
            Node { width: percent(100) }
            Children [
                @pane_header()
                Children [
                    Text("Dev Tools") ThemedText
                    --
                    Node { column_gap: px(6) }
                    TabList { activation: TabActivation::Automatic }
                    @selected_tab(#world_tab)
                    on(tablist_self_update)
                    on(select_tab)
                    Children [
                        #world_tab @tab_header("World", PanelTab::World)
                        --
                        @tab_header("Settings", PanelTab::Settings)
                    ]
                ]
                --
                @pane_body()
                PanelTab::World
                Node { row_gap: px(8) }
                InheritableThemeTextColor(tokens::TEXT_MAIN)
                Children [
                    @row("Fly", fly_control(flying))
                    --
                    @row("Time", time_control(time.fraction * 24.0))
                    --
                    @row("Day Speed", speed_control(time.speed))
                    --
                    Text({clock(&time)}) ThemedText ClockText
                ]
                --
                Node { display: Display::None }
                PanelTab::Settings
                Children [@settings_panel(settings, simulation)]
            ]
        ]
    }
}

fn selected_tab(tab: EntityTemplate) -> impl Scene {
    let tab = OptionTemplate::Some(tab);
    bsn! { SelectedTab({tab}) }
}

fn tab_header(label: &'static str, tab: PanelTab) -> impl Scene {
    bsn! {
        Tab
        tab
        FocusIndicator
        Node { padding: UiRect::axes(px(10), px(6)), align_items: AlignItems::Center }
        ThemeBackgroundColor({tab_color(tab == PanelTab::World)})
        Children [Text(label) ThemedText]
    }
}

fn tab_color(selected: bool) -> ThemeToken {
    if selected {
        tokens::BUTTON_PRIMARY_BG
    } else {
        tokens::BUTTON_BG
    }
}

fn select_tab(
    change: On<ValueChange<Option<Entity>>>,
    tabs: Query<(Entity, &PanelTab), With<Tab>>,
    mut panels: Query<
        (Option<&PanelTab>, &mut Node),
        (Without<Tab>, Or<(With<DevPanel>, With<PanelTab>)>),
    >,
    mut commands: Commands,
) {
    let Some((_, selected)) = change.value.and_then(|entity| tabs.get(entity).ok()) else {
        return;
    };
    for (entity, tab) in &tabs {
        commands
            .entity(entity)
            .insert(ThemeBackgroundColor(tab_color(tab == selected)));
    }
    for (tab, mut node) in &mut panels {
        if let Some(tab) = tab {
            node.display = if tab == selected {
                Display::Flex
            } else {
                Display::None
            };
        } else {
            node.width = px(if *selected == PanelTab::World {
                340.0
            } else {
                780.0
            });
        }
    }
}

fn row(label: &'static str, control: impl Scene) -> impl Scene {
    bsn! {
        Node {
            width: percent(100),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            column_gap: px(12),
        }
        Children [
            Text(label) ThemedText
            --
            @control
        ]
    }
}

fn fly_control(enabled: bool) -> impl Scene {
    let checked = enabled.then_some(bsn! { Checked });
    bsn! {
        @FeathersToggleSwitch
        @checked
        on(checkbox_self_update)
        on(set_flying)
    }
}

fn time_control(hours: f32) -> impl Scene {
    bsn! {
        @FeathersSlider { @min: 0.0, @max: 24.0 }
        Node { width: px(200) }
        SliderValue(hours)
        SliderStep(0.25)
        SliderPrecision(2)
        TimeSlider
        on(slider_self_update)
        on(set_time)
    }
}

fn speed_control(speed: f32) -> impl Scene {
    bsn! {
        @FeathersSlider { @min: 0.0, @max: MAX_DAY_SPEED }
        Node { width: px(200) }
        SliderValue({speed.clamp(0.0, MAX_DAY_SPEED)})
        SliderStep(1.0)
        SliderPrecision(0)
        on(slider_self_update)
        on(set_day_speed)
    }
}

fn clock(time: &TimeOfDay) -> String {
    let minutes = (time.fraction * 24.0 * 60.0) as u32;
    let state = if time.speed == 0.0 { " (frozen)" } else { "" };
    format!(
        "Day {}, {:02}:{:02}{state}",
        time.day,
        minutes / 60,
        minutes % 60
    )
}

fn set_flying(
    change: On<ValueChange<bool>>,
    player: Single<Entity, With<Player>>,
    mut commands: Commands,
) {
    if change.value {
        commands.entity(*player).insert(Flying);
    } else {
        commands.entity(*player).remove::<Flying>();
    }
}

fn set_time(change: On<ValueChange<f32>>, mut time: ResMut<TimeOfDay>) {
    time.fraction = (change.value / 24.0).clamp(0.0, 0.999_99);
}

fn set_day_speed(change: On<ValueChange<f32>>, mut time: ResMut<TimeOfDay>) {
    time.speed = change.value.round().clamp(0.0, MAX_DAY_SPEED);
}

/// Follows the running clock on the time slider and the clock text.
fn sync_clock(
    time: Res<TimeOfDay>,
    slider: Single<(Entity, &SliderValue), With<TimeSlider>>,
    mut text: Single<&mut Text, With<ClockText>>,
    mut commands: Commands,
) {
    if !time.is_changed() {
        return;
    }
    let hours = time.fraction * 24.0;
    let (slider, value) = *slider;
    // `SliderValue` is immutable: replacing it is how the widget hears of it.
    if (value.0 - hours).abs() >= 0.05 {
        commands.entity(slider).insert(SliderValue(hours));
    }
    let clock = clock(&time);
    if text.0 != clock {
        text.0 = clock;
    }
}
