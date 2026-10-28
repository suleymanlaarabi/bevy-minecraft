//! Application bootstrap and engine configuration.

mod assets;
#[cfg(feature = "dev")]
mod dev;
#[cfg(feature = "dev")]
mod flight_benchmark;
mod fonts;
#[cfg(not(feature = "dev"))]
mod fps;

pub use assets::{asset_directory, load_nearest};

use crate::{
    game::{GamePlugin, GameState},
    settings::GameSettingsPlugin,
    voxel::VoxelPlugin,
};
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
                    // All voxel CPU work shares this pool and its admission budget,
                    // leaving CPU time for rendering and parallel ECS systems.
                    async_compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                        min_threads: 1,
                        max_threads: 10,
                        percent: 0.50,
                        on_thread_spawn: None,
                        on_thread_destroy: None,
                    },
                    io: bevy::app::TaskPoolThreadAssignmentPolicy {
                        min_threads: 1,
                        max_threads: 2,
                        percent: 0.1,
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
            crate::content::MinecraftContentPlugin,
            crate::content::fluids::MinecraftFluidsPlugins,
            VoxelPlugin::default(),
            GamePlugin,
        ));
        #[cfg(feature = "dev")]
        app.add_plugins(dev::DevelopmentPlugin);
        #[cfg(not(feature = "dev"))]
        app.add_plugins(fps::FpsCounterPlugin);
        app.insert_resource(bevy::render::render_asset::RenderAssetBytesPerFrame::new(
            4 << 20,
        ))
        .insert_resource(UiTheme(create_dark_theme()))
        .insert_resource(ClearColor(Color::srgb_u8(15, 15, 18)))
        .add_systems(PreStartup, fonts::install)
        .add_systems(Startup, cpu_light_clustering)
        .add_systems(
            PostUpdate,
            fonts::style_new_text
                .after(bevy::ui::UiSystems::Propagate)
                .before(bevy::ui::UiSystems::Content),
        )
        .add_systems(
            Update,
            crate::game::handle_escape.run_if(in_state(GameState::Game)),
        );
    }
}

/// Hollow spawns no clustered objects (point, spot or rect lights, light probes or
/// clustered decals). GPU clustering would still run its passes, buffer uploads and
/// statistics readback for every camera each frame; CPU clustering of the single
/// cluster each camera requests has nothing to assign. Any future clustered light
/// remains correct, only unbinned.
pub(crate) fn cpu_light_clustering(
    settings: Option<ResMut<bevy::light::cluster::GlobalClusterSettings>>,
) {
    if let Some(mut settings) = settings {
        settings.gpu_clustering = None;
    }
}
