use super::{
    ChunkVoxels, VoxelChunk, VoxelCommandsExt, VoxelKind, VoxelWorld, streaming::PendingChunks,
};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode};
use bevy_voxel::character::CharacterMovement;
use bevy_voxel::render::build::{ChunkBuild, ChunkSections};
use bevy_voxel::streaming::AwaitingNeighbors;

use crate::{
    game::GameState,
    player::{Player, PlayerCamera},
};

use super::{StreamOffsets, StreamState};

const MAX_FRAME_SAMPLES: usize = 3_600;

pub(crate) use bevy_voxel::render::diagnostics::VoxelDiagnostics;
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
    let report = std::env::var_os("HOLLOW_DIAGNOSTICS").is_some()
        || std::env::var_os("HOLLOW_FLIGHT_BENCH").is_some();
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
            (
                report_voxel_diagnostics.after(record_voxel_frame),
                report_memory,
            )
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
        app.insert_resource(bevy::winit::WinitSettings::continuous())
            .insert_resource(VoxelBenchmark {
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
                    .before(bevy_voxel::character::CharacterSystems)
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
    let screenshot = if std::env::var_os("HOLLOW_BENCH_HEADLESS").is_some() {
        bevy::render::view::screenshot::Screenshot(target.clone())
    } else {
        bevy::render::view::screenshot::Screenshot::primary_window()
    };
    commands
        .spawn(screenshot)
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
    chunks: Query<
        (
            &ChunkVoxels,
            Has<bevy_voxel::render::streaming::RenderRequested>,
        ),
        With<VoxelChunk>,
    >,
    builds: Query<(), With<ChunkBuild>>,
    sections: Query<&ChunkSections>,
    pending: Res<PendingChunks>,
    simulation: Res<crate::simulation::SimulationDiagnostics>,
    creatures: Query<Has<crate::simulation::Simulating>, With<crate::creatures::Creature>>,
    windows: Query<&Window>,
    settings: Res<crate::settings::GraphicsSettings>,
) {
    diagnostics.elapsed += time.delta_secs_f64();
    if diagnostics.elapsed < 5.0 {
        return;
    }
    diagnostics.elapsed = 0.0;

    for window in &windows {
        info!(
            "graphics: physical={}x{} scale={} settings={:?}",
            window.physical_width(),
            window.physical_height(),
            window.scale_factor(),
            *settings
        );
    }
    let loaded = chunks.iter().count();
    let rendered = chunks.iter().filter(|(_, rendered)| *rendered).count();
    let building = builds.iter().count();
    let modified = chunks.iter().filter(|(chunk, _)| chunk.modified).count();
    let dirty_chunks = chunks
        .iter()
        .filter(|(chunk, rendered)| *rendered && chunk.dirty_sections != 0)
        .count();
    let queued = dirty_chunks.saturating_sub(building);
    let ready = rendered.saturating_sub(dirty_chunks);
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
    let (creature_count, active_creatures) = creatures
        .iter()
        .fold((0usize, 0usize), |(total, active), simulating| {
            (total + 1, active + usize::from(simulating))
        });
    info!(
        "creatures: total={creature_count} active={active_creatures} paths_pending={} oldest_wait_ticks={} queue_bytes={} search_table_bytes={} expansions={} probes={}/32768 spawn_refusals[cap,terrain,space,occupied,biome,light,distance]={:?}",
        simulation.navigation_pending,
        simulation.navigation_oldest_wait,
        simulation.navigation_queue_bytes,
        simulation.navigation_search_bytes,
        simulation.navigation_expansions,
        simulation.navigation_probes,
        simulation.spawn_rejections
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
    mut events: MessageReader<bevy_voxel::engine::streaming::TerrainPublished>,
    mut diagnostics: ResMut<VoxelDiagnostics>,
) {
    for event in events.read() {
        diagnostics.record_generation(event.elapsed);
    }
}

#[derive(Default)]
struct MemoryHistory {
    last: Option<std::time::Instant>,
    peak_rss: usize,
    peak_known: usize,
    allocations: bevy_voxel::core::sections::AllocationCensus,
}
fn report_memory(world: &mut World, mut history: Local<MemoryHistory>) {
    use bevy_voxel::render::diagnostics::memory::{
        MemoryProbe, ecs_memory, queued_payload_bytes, sample_due,
    };
    if !sample_due(&mut history.last) {
        return;
    }
    history.allocations.clear();
    let census = &mut history.allocations;
    let generation = hollow_worldgen::runtime::memory_census(world, census);
    for voxels in world.query::<&ChunkVoxels>().iter(world) {
        voxels.census(census);
    }
    if let Some(stored) = world.get_resource::<super::streaming::StoredChunks>() {
        for voxels in stored.values() {
            voxels.census(census);
        }
    }
    if let Some(light) = world
        .get_resource::<bevy_voxel::engine::lighting::runtime::LightingState<super::VoxelState>>()
    {
        light.census(census);
    }
    if let Some(translucent) = world.get_resource::<bevy_voxel::render::sort::TranslucentSections>()
    {
        translucent.census(census);
    }
    let model_catalog = super::render::models::memory_census(census);
    let shared = census.bytes;
    let section_records: usize = world
        .query::<&ChunkSections>()
        .iter(world)
        .map(|sections| std::mem::size_of_val(sections.sections.as_ref()))
        .sum();
    let queued: usize = world
        .query::<&ChunkBuild>()
        .iter(world)
        .map(ChunkBuild::ready_payload_bytes)
        .sum();
    let images: usize = world.get_resource::<Assets<Image>>().map_or(0, |images| {
        images
            .iter()
            .map(|(id, image)| {
                let bytes = image.data.as_ref().map_or(0, Vec::capacity);
                if bytes >= 1 << 20 {
                    info!(?id, size = ?image.texture_descriptor.size, usage = ?image.asset_usage,
                        bytes, "memory census: retained CPU image");
                }
                bytes
            })
            .sum()
    });
    let mesh_payload: usize = world.get_resource::<Assets<Mesh>>().map_or(0, |meshes| {
        meshes
            .iter()
            .map(|(_, mesh)| {
                mesh.try_attributes().map_or(0, |attributes| {
                    attributes
                        .map(|(_, values)| values.get_bytes().len())
                        .sum::<usize>()
                }) + mesh.try_indices().map_or(0, |indices| match indices {
                    bevy::mesh::Indices::U16(values) => values.capacity() * 2,
                    bevy::mesh::Indices::U32(values) => values.capacity() * 4,
                })
            })
            .sum()
    });
    let main_ecs = ecs_memory(world);
    let render = world
        .get_resource::<MemoryProbe>()
        .map(|probe| probe.0.lock().unwrap().clone())
        .unwrap_or_default();
    let queued_payload = queued + queued_payload_bytes(world);
    let mesher_scratch = bevy_voxel::render::meshing::scratch_bytes();
    let known = shared
        + mesher_scratch
        + section_records
        + generation.working_bytes
        + generation.metadata_lower_bound
        + generation.density_bytes[0]
        + generation.density_bytes[1]
        + queued_payload
        + images
        + mesh_payload
        + main_ecs.table_reserved
        + main_ecs.sparse_live_lower_bound
        + render.ecs.table_reserved
        + render.ecs.sparse_live_lower_bound
        + render.cpu_payload;
    let resident = process_memory();
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: mallinfo2 takes no pointers and only snapshots allocator counters.
        let allocator = unsafe { libc::mallinfo2() };
        info!(
            arena_bytes = allocator.arena,
            live_arena_bytes = allocator.uordblks,
            free_arena_bytes = allocator.fordblks,
            mmap_bytes = allocator.hblkhd,
            "memory census: glibc allocator; mapped bytes may be nonresident"
        );
    }
    history.peak_rss = history.peak_rss.max(resident[0].unwrap_or(0));
    history.peak_known = history.peak_known.max(known);
    // Logical allocations and resident pages are different quantities. The residual
    // also includes allocator slack, driver mappings, opaque task captures and catalogs.
    info!(rss_bytes = ?resident[0], anonymous_rss_bytes = ?resident[1], file_pss_bytes = ?resident[2], shared_pss_bytes = ?resident[3],
        peak_rss_bytes = history.peak_rss, known_allocation_bytes = known, peak_known_bytes = history.peak_known,
        rss_minus_known_estimate = ?resident[0].map(|rss| rss as i128 - known as i128),
        ecs_main = ?main_ecs, ecs_render = ?render.ecs,
        shared_allocation_bytes = shared, model_catalog = ?model_catalog, mesher_scratch_bytes = mesher_scratch, generation = ?generation, section_records_bytes = section_records,
        image_bytes = images, mesh_payload_bytes = mesh_payload, queued_geometry_payload_bytes = queued_payload,
        gpu_arena_live_bytes = render.arena_live, gpu_arena_reserved_bytes = render.arena_reserved,
        "memory census: allocation lower bounds; task captures, native structure caches, catalogs and driver memory remain unattributed; GPU bytes are separate");
}
fn process_memory() -> [Option<usize>; 4] {
    let mut result = [None; 4];
    #[cfg(target_os = "linux")]
    if let Ok(smaps) = std::fs::read_to_string("/proc/self/smaps_rollup") {
        for line in smaps.lines() {
            let mut words = line.split_whitespace();
            let index = match words.next() {
                Some("Rss:") => 0,
                Some("Anonymous:") => 1,
                Some("Pss_File:") => 2,
                Some("Pss_Shmem:") => 3,
                _ => continue,
            };
            result[index] = words
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .and_then(|kib| kib.checked_mul(1024));
        }
    }
    result
}
