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
                    // All voxel CPU work shares this pool and its admission budget.
                    // About 30% of the hardware threads, at background OS priority,
                    // leaves cores, SMT siblings and the shared CPU/iGPU power budget
                    // to the main, render and compute threads; voxel workers still
                    // absorb every idle cycle.
                    async_compute: bevy::app::TaskPoolThreadAssignmentPolicy {
                        min_threads: 1,
                        max_threads: 8,
                        percent: 0.3,
                        on_thread_spawn: Some(std::sync::Arc::new(voxel_worker_start)),
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
            })
            // Settings only select FXAA, MSAA and bloom (added below). The grouped SMAA,
            // TAA, CAS, motion blur, depth of field and lens effects would still run
            // their render systems for every view each frame.
            .disable::<bevy::anti_alias::AntiAliasPlugin>()
            .disable::<bevy::post_process::PostProcessPlugin>();
        if headless {
            plugins = plugins.disable::<bevy::winit::WinitPlugin>();
            app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
                core::time::Duration::ZERO,
            ));
        }
        app.add_plugins((
            plugins,
            (
                bevy::anti_alias::fxaa::FxaaPlugin,
                bevy::post_process::bloom::BloomPlugin,
                bevy::post_process::msaa_writeback::MsaaWritebackPlugin,
            ),
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
        // The frame schedules hold about a thousand systems that mostly finish in
        // microseconds or are skipped by their conditions. Dispatching each one to
        // the compute pool costs a task and a thread wake-up, far more than the
        // systems themselves; heavy systems parallelize internally instead.
        for (_, schedule) in app
            .world_mut()
            .resource_mut::<bevy::ecs::schedule::Schedules>()
            .iter_mut()
        {
            schedule.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
        }
        // Extraction blocks both worlds and is made of the same kind of tiny systems.
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render.edit_schedule(bevy::render::ExtractSchedule, |schedule| {
                schedule.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
            });
        }
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

/// Voxel workers run at nice 10 and, on hybrid CPUs, stay on the performance cores:
/// the scheduler preempts them whenever a frame thread wakes on their core, yet they
/// keep every otherwise idle cycle. Only the workers are placed; frame threads remain
/// unpinned.
fn voxel_worker_start() {
    background_priority();
    #[cfg(target_os = "linux")]
    pin_to_performance_cpus();
}

#[cfg(target_os = "linux")]
fn pin_to_performance_cpus() {
    use std::sync::OnceLock;
    static PERFORMANCE_CPUS: OnceLock<Option<libc::cpu_set_t>> = OnceLock::new();
    let Some(set) = PERFORMANCE_CPUS.get_or_init(performance_cpu_set) else {
        return;
    };
    // SAFETY: `sched_setaffinity` with pid 0 applies to the calling thread and only reads
    // the mask. A failure (for example a restricted container) keeps the default placement.
    unsafe {
        libc::sched_setaffinity(0, core::mem::size_of::<libc::cpu_set_t>(), set);
    }
}

/// CPU mask of the performance cores, from the per-core-type CPU lists that hybrid Intel
/// kernels expose. Machines without them yield `None` and keep the scheduler's placement.
#[cfg(target_os = "linux")]
fn performance_cpu_set() -> Option<libc::cpu_set_t> {
    let list = std::fs::read_to_string("/sys/devices/cpu_core/cpus").ok()?;
    // SAFETY: an all-zero `cpu_set_t` is a valid empty mask.
    let mut set: libc::cpu_set_t = unsafe { core::mem::zeroed() };
    for range in list.trim().split(',') {
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        let first: usize = first.trim().parse().ok()?;
        let last: usize = last.trim().parse().ok()?;
        for cpu in (first..=last).filter(|&cpu| cpu < libc::CPU_SETSIZE as usize) {
            // SAFETY: `cpu` is below CPU_SETSIZE, so the bit index stays inside the mask.
            unsafe { libc::CPU_SET(cpu, &mut set) };
        }
    }
    Some(set)
}

/// Voxel workers run at nice 10 (see `voxel_worker_start`).
fn background_priority() {
    // Linux nice values are per thread; elsewhere workers keep the default priority.
    #[cfg(target_os = "linux")]
    // SAFETY: `setpriority` only reads its scalar arguments. On Linux, who = 0 with
    // PRIO_PROCESS targets the calling thread, not the whole process.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
}
