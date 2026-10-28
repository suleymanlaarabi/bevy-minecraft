//! Opt-in, rendered exploration measurement: stationary, outbound, return, recovery.

use crate::{
    character::{CharacterMotion, Flying},
    player::{Player, PlayerCamera},
    settings::{DisplayMode, GraphicsSettings},
    voxel::{ChunkVoxels, VoxelChunk},
};
use bevy::{
    diagnostic::{DiagnosticPath, DiagnosticsStore},
    prelude::*,
    window::WindowResolution,
    winit::WinitSettings,
};
use hollow_engine::streaming::{PendingChunks, StreamState};
use hollow_features::motion::{FixedPosition, InterpolatePosition};
use hollow_render::{build::ChunkBuild, diagnostics::VoxelDiagnostics, streaming::RenderRequested};
use std::{
    fs::File,
    io::{BufWriter, Write},
    time::Instant,
};

const CPU_ACTIVE: DiagnosticPath = DiagnosticPath::const_new("voxel/cpu_active");
const CPU_PENDING: DiagnosticPath = DiagnosticPath::const_new("voxel/cpu_pending");
const REST_SECONDS: f64 = 20.0;

pub(super) struct FlightBenchmarkPlugin;

impl Plugin for FlightBenchmarkPlugin {
    fn build(&self, app: &mut App) {
        assert!(
            std::env::var_os("HOLLOW_VOXEL_BENCH").is_none(),
            "Choose one benchmark scenario"
        );
        assert!(
            std::env::var_os("HOLLOW_BENCH_HEADLESS").is_none(),
            "Flight measurements require a rendered window"
        );
        crate::world::register_flight_benchmark(app);
        let path =
            std::env::var_os("HOLLOW_FLIGHT_CSV").unwrap_or_else(|| "flight-benchmark.csv".into());
        let mut csv = BufWriter::new(File::create(&path).expect("Create flight benchmark CSV"));
        writeln!(csv, "seconds,phase,x,y,z,frame_ms,main_schedule_ms,loads_completed,builds_completed,load_wall_ms,mesh_worker_ms,mesh_apply_ms,uploaded_bytes,uploaded_meshes,cpu_active,cpu_pending,pending_write_bytes,world_phase,visibility_ms,translucent_sort_ms,visibility_columns,visibility_remaining,visibility_conservative")
            .expect("Write flight benchmark CSV header");
        info!(
            "flight benchmark: frame CSV at {}",
            std::path::Path::new(&path).display()
        );
        app.insert_resource(FlightBenchmark {
            csv,
            phase: Phase::Warmup,
            phase_elapsed: 0.0,
            flight_seconds: option("HOLLOW_FLIGHT_SECONDS", 60.0, 5.0, 600.0),
            speed: option("HOLLOW_FLIGHT_SPEED", 16.0, 1.0, 128.0) as f32,
            origin: None,
            start: Instant::now(),
            frame_start: Instant::now(),
            previous: None,
            main_ms: 0.0,
            counters: Counters::default(),
            samples: std::array::from_fn(|_| Vec::with_capacity(4096)),
            last_flush: Instant::now(),
            original_graphics: None,
        })
        .insert_resource(WinitSettings::continuous())
        .add_systems(Startup, start)
        .add_systems(PostStartup, resolution)
        .add_systems(First, record_frame)
        .add_systems(
            RunFixedMainLoop,
            fly.after(InterpolatePosition)
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                .run_if(crate::world::playing),
        )
        .add_systems(Last, finish_frame.after(crate::world::WorldSaveCapture));
    }
}

fn option(name: &str, default: f64, min: f64, max: f64) -> f64 {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && (min..=max).contains(value))
            .unwrap_or_else(|| panic!("{name} must be between {min} and {max}")),
        Err(_) => default,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Warmup,
    Stationary,
    Outbound,
    Return,
    Recovery,
    Done,
}

impl Phase {
    fn index(self) -> Option<usize> {
        match self {
            Self::Stationary => Some(0),
            Self::Outbound => Some(1),
            Self::Return => Some(2),
            Self::Recovery => Some(3),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Counters {
    loads: u64,
    builds: u64,
    load: f64,
    mesh: f64,
    apply: f64,
    visibility: f64,
    sort: f64,
    bytes: u64,
    meshes: u64,
}

#[derive(Resource)]
struct FlightBenchmark {
    csv: BufWriter<File>,
    phase: Phase,
    phase_elapsed: f64,
    flight_seconds: f64,
    speed: f32,
    origin: Option<Vec3>,
    start: Instant,
    frame_start: Instant,
    previous: Option<(Phase, Vec3)>,
    main_ms: f64,
    counters: Counters,
    samples: [Vec<(f64, f64)>; 4],
    last_flush: Instant,
    original_graphics: Option<GraphicsSettings>,
}

fn start(
    graphics: Res<GraphicsSettings>,
    mut benchmark: ResMut<FlightBenchmark>,
    mut commands: Commands,
) {
    let mut settings = graphics.clone();
    benchmark.original_graphics = Some(settings.clone());
    settings.view_distance = 12;
    settings.vsync = false;
    if std::env::var_os("HOLLOW_FLIGHT_RESOLUTION").is_some() {
        settings.display_mode = DisplayMode::Windowed;
    }
    info!("flight benchmark: seed=12 distance=12 target_frame_ms=4.167 graphics={settings:?}");
    commands.insert_resource(settings);
    if std::env::var_os("HOLLOW_FLIGHT_NO_MOBS").is_some() {
        commands.insert_resource(crate::creatures::spawning::CreaturePopulationLimit(0));
        info!("flight benchmark: creature spawning disabled");
    }
    if std::env::var_os("HOLLOW_FLIGHT_PAUSE_SIMULATION").is_some() {
        commands.insert_resource(crate::simulation::SimulationPaused);
        info!("flight benchmark: gameplay simulation paused for isolation");
    }
}

fn resolution(mut windows: Query<&mut Window>) {
    let Ok(value) = std::env::var("HOLLOW_FLIGHT_RESOLUTION") else {
        return;
    };
    let (width, height) = value
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .filter(|(w, h)| (320..=7680).contains(w) && (240..=4320).contains(h))
        .expect("HOLLOW_FLIGHT_RESOLUTION must be WIDTHxHEIGHT in physical pixels");
    for mut window in &mut windows {
        window.resolution = WindowResolution::new(width, height).with_scale_factor_override(1.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn fly(
    time: Res<Time<Real>>,
    mut time_of_day: ResMut<crate::sky::TimeOfDay>,
    mut benchmark: ResMut<FlightBenchmark>,
    mut players: Query<
        (
            Entity,
            &mut Transform,
            &mut FixedPosition,
            &mut CharacterMotion,
        ),
        With<Player>,
    >,
    mut cameras: Query<&mut PlayerCamera>,
    pending: Res<PendingChunks>,
    stream: Res<StreamState>,
    builds: Query<(), With<ChunkBuild>>,
    chunks: Query<&ChunkVoxels, (With<VoxelChunk>, With<RenderRequested>)>,
    mut commands: Commands,
) {
    if benchmark.phase == Phase::Done {
        return;
    }
    let Ok((entity, mut transform, mut position, mut motion)) = players.single_mut() else {
        return;
    };
    if benchmark.origin.is_none() {
        let origin = transform.translation + Vec3::Y * 40.0;
        benchmark.origin = Some(origin);
        benchmark.start = Instant::now();
        benchmark.phase_elapsed = 0.0;
        commands.entity(entity).insert(Flying);
        time_of_day.fraction = 0.5;
        time_of_day.speed = 0.0;
        info!(
            "flight benchmark: origin={origin:?} speed={} blocks/s leg={}s; warming up",
            benchmark.speed, benchmark.flight_seconds
        );
    }
    benchmark.phase_elapsed += time.delta_secs_f64();
    let duration = match benchmark.phase {
        Phase::Warmup => 10.0,
        Phase::Stationary | Phase::Recovery => REST_SECONDS,
        _ => benchmark.flight_seconds,
    };
    if benchmark.phase_elapsed >= duration {
        let ready = benchmark.phase != Phase::Warmup
            || (pending.is_empty()
                && stream.missing.is_empty()
                && builds.is_empty()
                && !chunks.is_empty()
                && chunks.iter().all(|chunk| chunk.dirty_sections == 0));
        if benchmark.phase == Phase::Warmup && !ready && benchmark.phase_elapsed > 180.0 {
            error!("flight benchmark: world did not settle within 180 seconds; no valid run");
            benchmark.csv.flush().expect("Flush flight CSV");
            benchmark.phase = Phase::Done;
            commands.write_message(AppExit::error());
            return;
        }
        if ready {
            benchmark.phase = match benchmark.phase {
                Phase::Warmup => Phase::Stationary,
                Phase::Stationary => Phase::Outbound,
                Phase::Outbound => Phase::Return,
                Phase::Return => Phase::Recovery,
                _ => Phase::Done,
            };
            benchmark.phase_elapsed = 0.0;
            info!("flight benchmark: phase={:?}", benchmark.phase);
            if benchmark.phase == Phase::Done {
                summarize(&mut benchmark);
                commands.insert_resource(benchmark.original_graphics.take().unwrap());
                crate::world::request_exit(&mut commands);
                return;
            }
        }
    }
    let distance = benchmark.speed * benchmark.flight_seconds as f32;
    let x = match benchmark.phase {
        Phase::Outbound => benchmark.speed * benchmark.phase_elapsed as f32,
        Phase::Return => distance - benchmark.speed * benchmark.phase_elapsed as f32,
        _ => 0.0,
    };
    transform.translation = benchmark.origin.unwrap() + Vec3::X * x;
    transform.rotation = Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
    for mut camera in &mut cameras {
        camera.pitch = -0.18;
    }
    // Scripted noclip travel keeps the route and real-time speed independent of
    // terrain/collisions and fixed-step catch-up. World simulation remains active.
    position.teleport(transform.translation);
    position.acknowledge(&transform);
    motion.velocity = default();
}

fn record_frame(
    mut benchmark: ResMut<FlightBenchmark>,
    voxel: Res<VoxelDiagnostics>,
    diagnostics: Res<DiagnosticsStore>,
    simulation: Res<crate::simulation::SimulationDiagnostics>,
    world_phase: Option<Res<State<crate::world::WorldSessionPhase>>>,
) {
    let now = Instant::now();
    let frame_ms = now.duration_since(benchmark.frame_start).as_secs_f64() * 1000.0;
    benchmark.frame_start = now;
    let current = Counters {
        loads: voxel.generated_chunks,
        builds: voxel.built_chunks,
        load: voxel.load_wall_seconds,
        mesh: voxel.mesh_worker_seconds,
        apply: voxel.mesh_apply_seconds,
        visibility: voxel.visibility_seconds,
        sort: voxel.translucent_sort_seconds,
        bytes: voxel.uploaded_bytes,
        meshes: voxel.uploaded_meshes,
    };
    // Sample the completed frame at the next First, pairing its full interval,
    // main schedule duration and counters, rather than shifting spikes by a frame.
    if let Some((phase, position)) = benchmark.previous
        && let Some(index) = phase.index()
    {
        let main_ms = benchmark.main_ms;
        benchmark.samples[index].push((frame_ms, main_ms));
        let elapsed = benchmark.start.elapsed().as_secs_f64();
        let active = diagnostics
            .get(&CPU_ACTIVE)
            .and_then(|d| d.value())
            .unwrap_or(0.0);
        let pending = diagnostics
            .get(&CPU_PENDING)
            .and_then(|d| d.value())
            .unwrap_or(0.0);
        let old = &benchmark.counters;
        let row = (
            current.loads - old.loads,
            current.builds - old.builds,
            (current.load - old.load) * 1000.0,
            (current.mesh - old.mesh) * 1000.0,
            (current.apply - old.apply) * 1000.0,
            current.bytes - old.bytes,
            current.meshes - old.meshes,
            (current.visibility - old.visibility) * 1000.0,
            (current.sort - old.sort) * 1000.0,
        );
        writeln!(benchmark.csv, "{elapsed:.6},{phase:?},{:.3},{:.3},{:.3},{frame_ms:.4},{main_ms:.4},{},{},{:.4},{:.4},{:.4},{},{},{active:.0},{pending:.0},{},{:?},{:.4},{:.4},{},{},{}",
            position.x, position.y, position.z, row.0, row.1, row.2, row.3, row.4, row.5, row.6,
            simulation.pending_write_bytes, world_phase.as_ref().map(|phase| *phase.get()), row.7, row.8,
            voxel.visibility_columns, voxel.visibility_remaining, u8::from(voxel.visibility_conservative))
            .expect("Write flight benchmark frame");
    }
    benchmark.counters = current;
    if benchmark.last_flush.elapsed().as_secs_f64() >= 1.0 {
        benchmark.csv.flush().expect("Flush flight benchmark CSV");
        benchmark.last_flush = now;
    }
}

fn finish_frame(mut benchmark: ResMut<FlightBenchmark>, players: Query<&Transform, With<Player>>) {
    benchmark.main_ms = benchmark.frame_start.elapsed().as_secs_f64() * 1000.0;
    benchmark.previous = players
        .single()
        .ok()
        .map(|transform| (benchmark.phase, transform.translation));
}

fn summarize(benchmark: &mut FlightBenchmark) {
    benchmark.csv.flush().expect("Flush flight benchmark CSV");
    for (phase, samples) in [
        Phase::Stationary,
        Phase::Outbound,
        Phase::Return,
        Phase::Recovery,
    ]
    .into_iter()
    .zip(&benchmark.samples)
    {
        if samples.is_empty() {
            continue;
        }
        let mut frames: Vec<_> = samples.iter().map(|s| s.0).collect();
        let mut main: Vec<_> = samples.iter().map(|s| s.1).collect();
        let fps = 1000.0 * frames.len() as f64 / frames.iter().sum::<f64>();
        let slow = frames.iter().filter(|&&ms| ms > 1000.0 / 240.0).count();
        frames.sort_unstable_by(f64::total_cmp);
        main.sort_unstable_by(f64::total_cmp);
        let p = |values: &[f64], fraction: f64| {
            values[((values.len() - 1) as f64 * fraction).round() as usize]
        };
        info!(
            "flight result: phase={phase:?} frames={} fps={fps:.1} frame_ms[p50={:.2} p95={:.2} p99={:.2} max={:.2}] main_schedule_ms[p50={:.2} p95={:.2}] over_4.167ms={:.1}% over_16.667ms={} over_33.333ms={}",
            frames.len(),
            p(&frames, 0.5),
            p(&frames, 0.95),
            p(&frames, 0.99),
            frames.last().unwrap(),
            p(&main, 0.5),
            p(&main, 0.95),
            slow as f64 * 100.0 / frames.len() as f64,
            frames.iter().filter(|&&ms| ms > 1000.0 / 60.0).count(),
            frames.iter().filter(|&&ms| ms > 1000.0 / 30.0).count()
        );
    }
}
