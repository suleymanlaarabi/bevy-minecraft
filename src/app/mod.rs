//! Application bootstrap and engine configuration.

mod assets;
#[cfg(feature = "dev")]
mod dev;

pub use assets::asset_directory;

use crate::{
    game::{GamePlugin, GameState},
    settings::GameSettingsPlugin,
    voxel::VoxelPlugin,
};
use avian3d::prelude::PhysicsPlugins;
use bevy::{
    feathers::{FeathersPlugins, dark_theme::create_dark_theme, theme::UiTheme},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, ExitCondition, WindowMode},
};

/// Composes the engine and gameplay plugins for the Hollow executable.
pub struct HollowPlugin;

impl Plugin for HollowPlugin {
    fn build(&self, app: &mut App) {
        let headless = cfg!(feature = "dev") && std::env::var_os("HOLLOW_BENCH_HEADLESS").is_some();
        let mut plugins = DefaultPlugins
            .set(TaskPoolPlugin {
                task_pool_options: TaskPoolOptions {
                    // Chunk meshing runs here; Bevy's default caps it at 4 threads.
                    // Meshing and colliders run here; the rest of the cores stay free for
                    // the main and render threads and Pumpkin's generation threads.
                    async_compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                        min_threads: 2,
                        max_threads: 6,
                        percent: 0.25,
                        on_thread_spawn: None,
                        on_thread_destroy: None,
                    },
                    ..default()
                },
            })
            .set(AssetPlugin {
                file_path: asset_directory().to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                primary_window: (!headless).then(|| Window {
                    mode: WindowMode::BorderlessFullscreen(MonitorSelection::Primary),
                    ..default()
                }),
                primary_cursor_options: Some(CursorOptions {
                    visible: true,
                    grab_mode: CursorGrabMode::None,
                    ..default()
                }),
                ..default()
            });
        if headless {
            plugins = plugins.disable::<bevy::winit::WinitPlugin>();
            app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                core::time::Duration::ZERO,
            ));
        }
        app.add_plugins((
            plugins,
            FeathersPlugins,
            GameSettingsPlugin,
            PhysicsPlugins::default(),
            VoxelPlugin::default(),
            GamePlugin,
        ));
        #[cfg(feature = "dev")]
        app.add_plugins(dev::DevelopmentPlugin);
        app.insert_resource(UiTheme(create_dark_theme()))
            .insert_resource(ClearColor(Color::srgb_u8(15, 15, 18)))
            .add_systems(
                Update,
                crate::game::handle_escape.run_if(in_state(GameState::Game)),
            );
    }
}
