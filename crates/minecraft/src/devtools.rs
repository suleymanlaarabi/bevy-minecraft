//! Developer panel, toggled with P: world controls and shared graphics settings.
//!
//! While it is open the cursor is free and the player neither moves nor interacts,
//! like with the inventory.

use bevy::{
    ecs::template::{EntityTemplate, OptionTemplate},
    feathers::{
        containers::{pane, pane_body, pane_header},
        controls::{ButtonVariant, FeathersButton, FeathersSlider, FeathersToggleSwitch},
        focus::FocusIndicator,
        theme::{InheritableThemeTextColor, ThemeBackgroundColor, ThemeToken, ThemedText},
        tokens,
    },
    prelude::*,
    ui::Checked,
    ui_widgets::{
        Activate, SelectedTab, SliderPrecision, SliderStep, SliderValue, Tab, TabActivation,
        TabList, ValueChange, checkbox_self_update, slider_self_update, tablist_self_update,
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

#[derive(Component, Clone, Default)]
struct CreatureSpawnMessage;

#[derive(Resource)]
pub(crate) struct CreatureSpawnStatus(pub(crate) &'static str);
impl Default for CreatureSpawnStatus {
    fn default() -> Self {
        Self("Choose a creature to spawn.")
    }
}

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
        app.init_resource::<CreatureSpawnStatus>()
            .add_observer(developer_spawn)
            .add_sub_state::<DevPanelState>()
            .add_systems(
                Update,
                toggle_panel
                    .run_if(crate::world::playing)
                    .run_if(in_state(InventoryState::Closed))
                    .run_if(any_with_component::<Player>),
            )
            .add_systems(
                Update,
                spawn_creature_keys
                    .run_if(crate::world::playing)
                    .run_if(in_state(DevPanelState::Open)),
            )
            // Keys held when the panel opens must not keep the player moving.
            .add_systems(
                OnEnter(DevPanelState::Open),
                (clear_player_input, spawn_panel),
            )
            .add_systems(
                Update,
                (
                    sync_clock.after(crate::sky::SkySystems::Advance),
                    sync_creature_spawn_status,
                )
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
                    --
                    Text("Creatures: C cow / V pig (F6 also works)") ThemedText
                    --
                    @FeathersButton { @caption: bsn! { Text("Spawn Cow") ThemedText }, @variant: ButtonVariant::Normal }
                    on(spawn_cow_button)
                    --
                    @FeathersButton { @caption: bsn! { Text("Spawn Pig") ThemedText }, @variant: ButtonVariant::Normal }
                    on(spawn_pig_button)
                    --
                    @FeathersButton { @caption: bsn! { Text("Spawn 10000 Animals") ThemedText }, @variant: ButtonVariant::Normal }
                    on(spawn_population_button)
                    --
                    Text("Choose a creature to spawn.") ThemedText CreatureSpawnMessage
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

#[derive(Event)]
struct DeveloperSpawn(crate::creatures::SpeciesId);

fn spawn_cow_button(_: On<Activate>, mut commands: Commands) {
    commands.trigger(DeveloperSpawn(crate::creatures::SpeciesId(
        pumpkin_data::entity::EntityType::COW.id,
    )));
}
fn spawn_pig_button(_: On<Activate>, mut commands: Commands) {
    commands.trigger(DeveloperSpawn(crate::creatures::SpeciesId(
        pumpkin_data::entity::EntityType::PIG.id,
    )));
}
fn spawn_population_button(_: On<Activate>, mut commands: Commands) {
    commands.trigger(crate::creatures::SpawnCreaturePopulation(10_000));
}

fn spawn_creature_keys(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    let native = if keys.just_pressed(KeyCode::KeyV)
        || (keys.just_pressed(KeyCode::F6)
            && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)))
    {
        &pumpkin_data::entity::EntityType::PIG
    } else if keys.just_pressed(KeyCode::KeyC) || keys.just_pressed(KeyCode::F6) {
        &pumpkin_data::entity::EntityType::COW
    } else {
        return;
    };
    commands.trigger(DeveloperSpawn(crate::creatures::SpeciesId(native.id)));
}

/// Check a few nearby positions only when a developer requests an animal.
fn developer_spawn(
    event: On<DeveloperSpawn>,
    player: Query<
        (
            &hollow_features::motion::FixedPosition,
            &crate::character::CharacterBody,
            &crate::character::CharacterState,
            &Transform,
        ),
        With<Player>,
    >,
    creatures: Query<
        (
            &hollow_features::motion::FixedPosition,
            &crate::character::CharacterBody,
            &crate::character::CharacterState,
        ),
        With<crate::creatures::Creature>,
    >,
    columns: Res<crate::block_behavior::containers::GameplayColumns>,
    members: Query<&crate::simulation::SimulatedEntities>,
    catalog: Res<crate::creatures::CreatureCatalog>,
    world: crate::voxel::VoxelWorld,
    mut terrain: Local<crate::character::Terrain>,
    mut status: ResMut<CreatureSpawnStatus>,
    mut commands: Commands,
) {
    let Ok((position, player_body, player_state, transform)) = player.single() else {
        return;
    };
    let Some(profile) = catalog.get(event.0) else {
        return;
    };
    let player_bounds = player_body.aabb(position.current, player_state.pose);
    let forward = transform
        .forward()
        .as_vec3()
        .with_y(0.0)
        .normalize_or_zero();
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let player_feet = position.current.as_vec3() - Vec3::Y * player_body.origin_height;
    for (distance, sideways) in [(3.0, 0.0), (2.5, 1.0), (2.5, -1.0), (4.0, 0.0)] {
        let point = player_feet + forward * distance + right * sideways;
        if !world.collision_ready(point) {
            continue;
        }
        for offset in [0.0, 0.5, 1.0, -0.5, -1.0, 1.5, 2.0, -1.5, -2.0, 3.0] {
            let feet = point.with_y(player_feet.y.floor() + offset);
            let precise_origin =
                feet.as_dvec3() + bevy::math::DVec3::Y * profile.body.origin_height as f64;
            let bounds = profile
                .body
                .aabb(precise_origin, crate::character::Pose::Standing);
            terrain.gather(
                &world,
                bounds.expanded_towards(bevy::math::DVec3::NEG_Y * 0.05),
            );
            if terrain.collides(&bounds)
                || terrain.has_fluid(&bounds)
                || terrain
                    .collide_box(bounds, bevy::math::DVec3::NEG_Y * 0.05)
                    .y
                    < -0.049
                || bounds.intersects(&player_bounds)
            {
                continue;
            }
            let reach = bevy::math::DVec3::splat(catalog.maximum_half_width() as f64);
            let min = crate::voxel::chunk_at((bounds.min - reach).as_vec3());
            let max = crate::voxel::chunk_at((bounds.max + reach).as_vec3());
            let occupied = (min.x..=max.x).any(|x| {
                (min.y..=max.y).any(|z| {
                    columns
                        .get(&IVec2::new(x, z))
                        .and_then(|owner| members.get(*owner).ok())
                        .is_some_and(|entities| {
                            entities.iter().any(|entity| {
                                creatures.get(entity).is_ok_and(|(other, body, state)| {
                                    bounds.intersects(&body.aabb(other.current, state.pose))
                                })
                            })
                        })
                })
            });
            if occupied {
                continue;
            }
            status.0 = "Spawn requested...";
            commands.trigger(crate::creatures::SpawnCreature {
                species: event.0,
                feet,
                source: crate::creatures::SpawnSource::Developer,
                population_center: position.current.as_vec3(),
            });
            return;
        }
    }
    status.0 = "No free ground nearby. Move to an open area.";
    warn!("Could not find free ground nearby for the creature.");
}

fn sync_creature_spawn_status(
    status: Res<CreatureSpawnStatus>,
    mut messages: Query<(Ref<CreatureSpawnMessage>, &mut Text)>,
) {
    for (message, mut text) in &mut messages {
        if (status.is_changed() || message.is_added()) && text.0 != status.0 {
            text.0 = status.0.into();
        }
    }
}
