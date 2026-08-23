use avian3d::prelude::*;
use bevy::{
    dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, WindowMode},
};
use hollow::{
    game::{GamePlugin, GameState},
    voxel::{VoxelPlugin, VoxelSettings},
};

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
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
            PhysicsPlugins::default(),
            VoxelPlugin::new(VoxelSettings {
                view_distance: 20,
                ..default()
            }),
            FpsOverlayPlugin {
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
            },
            GamePlugin,
        ))
        .insert_resource(ClearColor(Color::srgb_u8(15, 15, 18)))
        .add_systems(Update, handle_escape.run_if(in_state(GameState::Game)))
        .run();
}

fn handle_escape(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        next_state.set(GameState::Menu);
    }
}
