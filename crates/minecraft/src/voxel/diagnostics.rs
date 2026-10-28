use super::{
    ChunkVoxels, VoxelChunk, VoxelCommandsExt, VoxelKind, VoxelWorld,
    streaming::{AwaitingNeighbors, PendingChunks},
};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode};
use hollow_render::build::{ChunkBuild, ChunkSections};

use crate::{
    character::CharacterMovement,
    game::GameState,
    player::{Player, PlayerCamera},
};

use super::{StreamOffsets, StreamState};

const MAX_FRAME_SAMPLES: usize = 3_600;

pub(crate) use hollow_render::diagnostics::VoxelDiagnostics;
#[derive(Resource, Default)]
struct VoxelBenchmark {
    simplified: bool,
    phase: BenchmarkPhase,
    elapsed: f64,
    configured: bool,
    edit_timer: f64,
    edited: Option<(IVec3, VoxelKind)>,
    removed: bool,
    edits: usize,
    movement_start: Vec3,
    generated_at_start: u64,
}

#[derive(SystemParam)]
struct VoxelBenchmarkWorld<'w, 's> {
    offsets: Res<'w, StreamOffsets>,
    pending: Res<'w, PendingChunks>,
    chunks: Query<'w, 's, &'static ChunkVoxels, With<VoxelChunk>>,
    meshed: Query<'w, 's, &'static ChunkVoxels, (With<VoxelChunk>, Without<AwaitingNeighbors>)>,
    builds: Query<'w, 's, (), With<ChunkBuild>>,
    player: Query<'w, 's, &'static Transform, With<Player>>,
}

#[derive(Default, PartialEq, Eq)]
enum BenchmarkPhase {
    #[default]
    Waiting,
    Warmup,
    Stationary,
    Moving,
    Editing,
    Done,
}

pub(crate) fn register(app: &mut App) {
    let report = std::env::var_os("HOLLOW_DIAGNOSTICS").is_some();
    let benchmark = std::env::var_os("HOLLOW_VOXEL_BENCH").is_some();
    if report || benchmark {
        app.init_resource::<VoxelDiagnostics>().add_systems(
            Update,
            (record_voxel_frame, record_generation).run_if(in_state(GameState::Game)),
        );
    }
    if report {
        app.add_systems(
            Update,
            report_voxel_diagnostics
                .after(record_voxel_frame)
                .run_if(in_state(GameState::Game)),
        );
    }
    if std::env::var_os("HOLLOW_SCREENSHOT_PATH").is_some() {
        app.add_systems(
            Update,
            capture_debug_frame.run_if(in_state(GameState::Game)),
        );
    }
    if benchmark {
        app.insert_resource(VoxelBenchmark {
            simplified: std::env::var_os("HOLLOW_VOXEL_BENCH_SIMPLIFIED").is_some(),
            ..default()
        })
        .add_systems(OnExit(GameState::Game), reset_voxel_benchmark)
        .add_systems(
            Update,
            (configure_voxel_benchmark, run_voxel_benchmark)
                .chain()
                .after(record_voxel_frame)
                .run_if(in_state(GameState::Game))
                // The player spawns once its spawn point search finishes.
                .run_if(any_with_component::<PlayerCamera>),
        )
        .add_systems(
            FixedUpdate,
            benchmark_movement
                .before(crate::character::CharacterSystems)
                .run_if(in_state(GameState::Game)),
        );
        if std::env::var_os("HOLLOW_BENCH_HEADLESS").is_some() {
            app.add_systems(
                PostUpdate,
                setup_headless_targets.before(bevy::camera::CameraUpdateSystems),
            );
        }
    }
}

fn capture_debug_frame(
    time: Res<Time<Real>>,
    cameras: Query<(&bevy::camera::RenderTarget, &GlobalTransform), With<PlayerCamera>>,
    mut captured: Local<bool>,
    mut commands: Commands,
) {
    if *captured || time.elapsed_secs() < 12.0 {
        return;
    }
    let Ok((target, transform)) = cameras.single() else {
        return;
    };
    let Some(path) = std::env::var_os("HOLLOW_SCREENSHOT_PATH") else {
        return;
    };
    info!("debug capture: camera at {:?}", transform.translation());
    commands
        .spawn(bevy::render::view::screenshot::Screenshot(target.clone()))
        .observe(bevy::render::view::screenshot::save_to_disk(
            std::path::PathBuf::from(path),
        ));
    *captured = true;
}

fn setup_headless_targets(
    mut commands: Commands,
    cameras: Query<Entity, Added<PlayerCamera>>,
    mut images: ResMut<Assets<Image>>,
) {
    for entity in &cameras {
        let target = images.add(Image::new_target_texture(
            1920,
            1080,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        commands
            .entity(entity)
            .insert(bevy::camera::RenderTarget::Image(target.into()));
        info!("voxel benchmark: rendering offscreen at 1920x1080");
    }
}

fn reset_voxel_benchmark(mut benchmark: ResMut<VoxelBenchmark>, mut commands: Commands) {
    commands.remove_resource::<crate::simulation::SimulationPaused>();
    *benchmark = VoxelBenchmark {
        simplified: benchmark.simplified,
        ..default()
    };
}

fn record_voxel_frame(time: Res<Time<Real>>, mut diagnostics: ResMut<VoxelDiagnostics>) {
    diagnostics
        .frames_ms
        .push_back(time.delta_secs_f64() * 1_000.0);
    if diagnostics.frames_ms.len() > MAX_FRAME_SAMPLES {
        diagnostics.frames_ms.pop_front();
    }
}

#[allow(clippy::too_many_arguments)]
fn configure_voxel_benchmark(
    mut windows: Query<&mut Window>,
    cameras: Query<Entity, With<PlayerCamera>>,
    mut lights: Query<&mut DirectionalLight>,
    mut offsets: ResMut<StreamOffsets>,
    mut stream_state: ResMut<StreamState>,
    mut benchmark: ResMut<VoxelBenchmark>,
    settings: Res<crate::settings::GraphicsSettings>,
    mut commands: Commands,
) {
    if benchmark.configured {
        return;
    }
    benchmark.configured = true;
    info!("voxel benchmark graphics: {:?}", *settings);
    if benchmark.simplified {
        for mut window in &mut windows {
            window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Primary);
            window.present_mode = PresentMode::AutoNoVsync;
        }
        for entity in &cameras {
            commands.entity(entity).insert(Msaa::Off);
        }
        for mut light in &mut lights {
            light.shadow_maps_enabled = false;
        }
        commands.insert_resource(crate::simulation::SimulationPaused);
        *offsets = StreamOffsets::new(21);
        stream_state.center = None;
        info!("voxel benchmark simplified: radius=21, AA=off, shadows=off, physics=paused");
    } else {
        info!(
            "voxel benchmark gameplay: current graphics settings and active physics, radius={}",
            offsets.radius
        );
    }
}

fn benchmark_movement(
    benchmark: Res<VoxelBenchmark>,
    time: Res<Time<Fixed>>,
    mut players: Query<(&mut CharacterMovement, &mut Transform), With<Player>>,
) {
    // The benchmark drives the player until it finishes, then hands control back.
    if benchmark.phase == BenchmarkPhase::Done {
        return;
    }
    for (mut movement, mut transform) in &mut players {
        *movement = CharacterMovement::default();
        if benchmark.phase == BenchmarkPhase::Moving {
            if benchmark.simplified {
                transform.translation.x += 8.0 * time.delta_secs();
            } else {
                movement.direction = Vec3::X;
                movement.look_direction = Vec3::X;
                movement.sprint = true;
            }
        }
    }
}

fn run_voxel_benchmark(
    time: Res<Time<Real>>,
    world: VoxelBenchmarkWorld,
    mut benchmark: ResMut<VoxelBenchmark>,
    mut diagnostics: ResMut<VoxelDiagnostics>,
    voxel_world: VoxelWorld,
    mut commands: Commands,
) {
    let loaded = world.chunks.iter().count();
    let settled = benchmark.phase == BenchmarkPhase::Waiting
        && loaded == world.offsets.len()
        && world.pending.is_empty()
        && world.builds.is_empty()
        && world.meshed.iter().all(|chunk| chunk.dirty_sections == 0);
    let delta = time.delta_secs_f64();

    match benchmark.phase {
        BenchmarkPhase::Waiting if settled => {
            benchmark.phase = BenchmarkPhase::Warmup;
            benchmark.elapsed = 0.0;
            diagnostics.reset_frames();
            info!("voxel benchmark: world settled with {loaded} chunks; warming up for 5 seconds");
        }
        BenchmarkPhase::Warmup => {
            benchmark.elapsed += delta;
            if benchmark.elapsed >= 5.0 {
                benchmark.phase = BenchmarkPhase::Stationary;
                benchmark.elapsed = 0.0;
                diagnostics.reset_frames();
                info!("voxel benchmark: stationary sample started");
            }
        }
        BenchmarkPhase::Stationary => {
            benchmark.elapsed += delta;
            if benchmark.elapsed >= 20.0 {
                let (p50, p95, p99) = diagnostics.frame_percentiles();
                info!(
                    "voxel benchmark stationary: frame_ms[p50={p50:.2} p95={p95:.2} p99={p99:.2}]"
                );
                benchmark.phase = BenchmarkPhase::Moving;
                benchmark.movement_start = world
                    .player
                    .iter()
                    .next()
                    .map_or(Vec3::ZERO, |transform| transform.translation);
                benchmark.generated_at_start = diagnostics.generated_chunks;
                benchmark.elapsed = 0.0;
                diagnostics.reset_frames();
                info!("voxel benchmark: movement sample started");
            }
        }
        BenchmarkPhase::Moving => {
            benchmark.elapsed += delta;
            if benchmark.elapsed >= 20.0 {
                let (p50, p95, p99) = diagnostics.frame_percentiles();
                info!("voxel benchmark movement: frame_ms[p50={p50:.2} p95={p95:.2} p99={p99:.2}]");
                let displacement = world.player.iter().next().map_or(Vec3::ZERO, |transform| {
                    transform.translation - benchmark.movement_start
                });
                info!(
                    "voxel benchmark movement workload: distance={:.2} generated={}",
                    Vec2::new(displacement.x, displacement.z).length(),
                    diagnostics.generated_chunks - benchmark.generated_at_start
                );
                benchmark.phase = BenchmarkPhase::Editing;
                benchmark.elapsed = 0.0;
                diagnostics.reset_frames();
                info!("voxel benchmark: mining/placement sample started");
            }
        }
        BenchmarkPhase::Editing => {
            benchmark.elapsed += delta;
            benchmark.edit_timer += delta;
            if benchmark.edit_timer >= 0.25 {
                benchmark.edit_timer -= 0.25;
                if benchmark.edited.is_none()
                    && let Some(transform) = world.player.iter().next()
                {
                    for depth in 1..=4 {
                        let position =
                            transform.translation.floor().as_ivec3() + IVec3::new(3, -depth, 0);
                        if let Some(kind) =
                            voxel_world.get(position).filter(|kind| kind.is_breakable())
                        {
                            benchmark.edited = Some((position, kind));
                            break;
                        }
                    }
                }
                if let Some((position, kind)) = benchmark.edited {
                    commands.set_voxel(
                        position,
                        if benchmark.removed {
                            kind
                        } else {
                            VoxelKind::Air
                        },
                    );
                    benchmark.removed = !benchmark.removed;
                    benchmark.edits += 1;
                }
            }
            if benchmark.elapsed >= 20.0 {
                if let Some((position, kind)) = benchmark.edited {
                    commands.set_voxel(position, kind);
                }
                let (p50, p95, p99) = diagnostics.frame_percentiles();
                info!(
                    "voxel benchmark editing: edits={} frame_ms[p50={p50:.2} p95={p95:.2} p99={p99:.2}]",
                    benchmark.edits
                );
                benchmark.phase = BenchmarkPhase::Done;
                if std::env::var_os("HOLLOW_BENCH_EXIT").is_some() {
                    crate::world::request_exit(&mut commands);
                }
            }
        }
        BenchmarkPhase::Waiting | BenchmarkPhase::Done => {}
    }
}

fn report_voxel_diagnostics(
    time: Res<Time<Real>>,
    mut diagnostics: ResMut<VoxelDiagnostics>,
    chunks: Query<&ChunkVoxels, With<VoxelChunk>>,
    builds: Query<(), With<ChunkBuild>>,
    sections: Query<&ChunkSections>,
    pending: Res<PendingChunks>,
    simulation: Res<crate::simulation::SimulationDiagnostics>,
) {
    diagnostics.elapsed += time.delta_secs_f64();
    if diagnostics.elapsed < 5.0 {
        return;
    }
    diagnostics.elapsed = 0.0;

    let loaded = chunks.iter().count();
    let building = builds.iter().count();
    let modified = chunks.iter().filter(|chunk| chunk.modified).count();
    let dirty_chunks = chunks
        .iter()
        .filter(|chunk| chunk.dirty_sections != 0)
        .count();
    let queued = dirty_chunks.saturating_sub(building);
    let ready = loaded.saturating_sub(dirty_chunks);
    let frames: Vec<_> = diagnostics.frames_ms.iter().copied().collect();
    let average_vertices = diagnostics
        .built_vertices
        .checked_div(diagnostics.built_chunks)
        .unwrap_or(0);
    let average_triangles = diagnostics
        .built_triangles
        .checked_div(diagnostics.built_chunks)
        .unwrap_or(0);

    info!(
        "voxel: loaded={loaded} ready={ready} pending={} building={building} queued={queued} section_meshes={} modified={modified} frame_ms[p50={:.2} p95={:.2} p99={:.2}] generation_ms[p50={:.2} p95={:.2}] meshing_ms[p50={:.2} p95={:.2}] generated={} built={} avg_vertices={} avg_triangles={}",
        pending.len(),
        sections
            .iter()
            .map(ChunkSections::mesh_count)
            .sum::<usize>(),
        percentile(&frames, 0.50),
        percentile(&frames, 0.95),
        percentile(&frames, 0.99),
        percentile(&diagnostics.generation_ms, 0.50),
        percentile(&diagnostics.generation_ms, 0.95),
        percentile(&diagnostics.meshing_ms, 0.50),
        percentile(&diagnostics.meshing_ms, 0.95),
        diagnostics.generated_chunks,
        diagnostics.built_chunks,
        average_vertices,
        average_triangles,
    );

    info!(
        "simulation: active={} ready_cells_and_heads={} suspended={} scheduled={}/2048 neighbors={}/4096 mutations={}/4096 random={}/8192 max_lateness={} pending_write_bytes={}",
        simulation.active_columns,
        simulation.ready_work,
        simulation.suspended_work,
        simulation.scheduled_ticks,
        simulation.neighbor_cells,
        simulation.modified_cells,
        simulation.random_attempts,
        simulation.maximum_lateness,
        simulation.pending_write_bytes
    );
    diagnostics.generation_ms.clear();
    diagnostics.meshing_ms.clear();
}

fn percentile(samples: &[f64], percentile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted[index]
}

fn record_generation(
    mut events: MessageReader<hollow_engine::streaming::TerrainPublished>,
    mut diagnostics: ResMut<VoxelDiagnostics>,
) {
    for event in events.read() {
        diagnostics.record_generation(event.elapsed);
    }
}
