use avian3d::prelude::*;
#[cfg(feature = "dev")]
use bevy::dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig};
use bevy::{
    camera::MainPassResolutionOverride,
    prelude::*,
    render::{Extract, RenderApp, sync_world::RenderEntity},
    settings::SettingsPlugin,
    window::{CursorGrabMode, CursorOptions, WindowMode, WindowResolution},
};
use hollow::{
    game::{GamePlugin, GameState},
    player::PlayerCamera,
    settings::GraphicsSettings,
    voxel::{VoxelPlugin, VoxelSettings},
};

fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                mode: WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
                resolution: WindowResolution::new(1920, 1080),
                ..default()
            }),
            primary_cursor_options: Some(CursorOptions {
                visible: true,
                grab_mode: CursorGrabMode::None,
                ..default()
            }),
            ..default()
        }),
        PhysicsPlugins::default(),
        VoxelPlugin::new(VoxelSettings {
            view_distance: 20,
            ..default()
        }),
        GamePlugin,
    ));
    app.register_type::<GraphicsSettings>();
    app.add_plugins(SettingsPlugin::new("org.hollow.game"));
    add_dev_tools(&mut app);
    app.insert_resource(ClearColor(Color::srgb_u8(15, 15, 18)))
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
