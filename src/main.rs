use avian3d::prelude::*;
#[cfg(feature = "dev")]
use bevy::dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig};
use bevy::{
    feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, ExitCondition, WindowMode},
};
use hollow::{
    game::{GamePlugin, GameState},
    settings::GameSettingsPlugin,
    voxel::VoxelPlugin,
};

fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            exit_condition: ExitCondition::DontExit,
            primary_window: Some(Window {
                mode: WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
                ..default()
            }),
            primary_cursor_options: Some(CursorOptions {
                visible: true,
                grab_mode: CursorGrabMode::None,
                ..default()
            }),
            ..default()
        }),
        FeathersPlugins,
        GameSettingsPlugin,
        PhysicsPlugins::default(),
        VoxelPlugin::default(),
        GamePlugin,
    ));
    add_dev_tools(&mut app);
    app.insert_resource(UiTheme(create_dark_theme()))
        .insert_resource(ClearColor(Color::srgb_u8(15, 15, 18)))
        .add_systems(Update, handle_escape.run_if(in_state(GameState::Game)))
        .run();
}

#[cfg(feature = "dev")]
fn add_dev_tools(app: &mut App) {
    app.add_plugins(FpsOverlayPlugin {
        config: FpsOverlayConfig {
            text_config: TextFont {
                font_size: FontSize::Px(42.0),
                ..default()
            },
            text_color: Color::WHITE,
            refresh_interval: core::time::Duration::from_millis(100),
            enabled: true,
            frame_time_graph_config: FrameTimeGraphConfig {
                enabled: true,
                min_fps: 30.0,
                target_fps: 120.0,
            },
        },
    });
}

#[cfg(not(feature = "dev"))]
fn add_dev_tools(_app: &mut App) {}

fn handle_escape(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        next_state.set(GameState::Menu);
    }
}
