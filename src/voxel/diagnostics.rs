use super::{
    ChunkVoxels, VoxelChunk,
    rebuild::ChunkBuild,
    regions::{RenderRegions, VoxelRenderRegion},
    streaming::PendingChunks,
};
use avian3d::{prelude::Physics, schedule::PhysicsTime};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode};
use std::{collections::VecDeque, time::Duration};

use crate::{
    game::GameState,
    player::{Player, PlayerCamera},
};

use super::{StreamOffsets, StreamState};

const MAX_FRAME_SAMPLES: usize = 3_600;

#[derive(Resource, Default)]
pub(crate) struct VoxelDiagnostics {
    frames_ms: VecDeque<f64>,
    generation_ms: Vec<f64>,
    meshing_ms: Vec<f64>,
    collider_ms: Vec<f64>,
    generated_chunks: u64,
    built_chunks: u64,
    built_vertices: u64,
    built_triangles: u64,
    elapsed: f64,
}

impl VoxelDiagnostics {
    pub(crate) fn record_generation(&mut self, elapsed: Duration) {
        self.generated_chunks += 1;
        self.generation_ms.push(elapsed.as_secs_f64() * 1_000.0);
    }

    pub(crate) fn record_build(
        &mut self,
        meshing: Duration,
        collider: Duration,
        vertices: usize,
        triangles: usize,
    ) {
        self.built_chunks += 1;
        self.built_vertices += vertices as u64;
        self.built_triangles += triangles as u64;
        self.meshing_ms.push(meshing.as_secs_f64() * 1_000.0);
        self.collider_ms.push(collider.as_secs_f64() * 1_000.0);
    }

    fn reset_frames(&mut self) {
        self.frames_ms.clear();
    }

    fn frame_percentiles(&self) -> (f64, f64, f64) {
        let frames: Vec<_> = self.frames_ms.iter().copied().collect();
        (
            percentile(&frames, 0.50),
            percentile(&frames, 0.95),
            percentile(&frames, 0.99),
        )
    }
}

#[derive(Resource, Default)]
struct VoxelBenchmark {
    phase: BenchmarkPhase,
    elapsed: f64,
}

#[derive(SystemParam)]
struct VoxelBenchmarkWorld<'w, 's> {
    offsets: Res<'w, StreamOffsets>,
    pending: Res<'w, PendingChunks>,
    render_regions: Res<'w, RenderRegions>,
    chunks: Query<'w, 's, (), With<VoxelChunk>>,
    builds: Query<'w, 's, (), With<ChunkBuild>>,
    player: Query<'w, 's, &'static mut Transform, With<Player>>,
}

#[derive(Default, PartialEq, Eq)]
enum BenchmarkPhase {
    #[default]
    Waiting,
    Warmup,
    Stationary,
    Moving,
    Done,
}

pub(crate) fn register(app: &mut App) {
    if std::env::var_os("HOLLOW_DIAGNOSTICS").is_some() {
        app.init_resource::<VoxelDiagnostics>().add_systems(
            Update,
            report_voxel_diagnostics.run_if(in_state(GameState::Game)),
        );
    }
    if std::env::var_os("HOLLOW_VOXEL_BENCH").is_some() {
        app.init_resource::<VoxelDiagnostics>()
            .init_resource::<VoxelBenchmark>()
            .add_systems(OnEnter(GameState::Game), configure_voxel_benchmark)
            .add_systems(
                Update,
                run_voxel_benchmark.run_if(in_state(GameState::Game)),
            );
    }
}

fn configure_voxel_benchmark(
    mut windows: Query<&mut Window>,
    cameras: Query<Entity, With<PlayerCamera>>,
    mut lights: Query<&mut DirectionalLight>,
    mut physics_time: ResMut<Time<Physics>>,
    mut offsets: ResMut<StreamOffsets>,
    mut stream_state: ResMut<StreamState>,
    mut commands: Commands,
) {
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
    physics_time.pause();
    *offsets = StreamOffsets::new(21);
    stream_state.center = None;
    info!("voxel benchmark: native fullscreen, radius=21, AA=off, shadows=off, physics=paused");
}

fn run_voxel_benchmark(
    time: Res<Time<Real>>,
    mut world: VoxelBenchmarkWorld,
    mut benchmark: ResMut<VoxelBenchmark>,
    mut diagnostics: ResMut<VoxelDiagnostics>,
) {
    let loaded = world.chunks.iter().count();
    let settled = loaded == world.offsets.len()
        && world.pending.is_empty()
        && world.builds.is_empty()
        && world.render_regions.is_settled();
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
                benchmark.elapsed = 0.0;
                diagnostics.reset_frames();
                info!("voxel benchmark: movement sample started at 8 world units/second");
            }
        }
        BenchmarkPhase::Moving => {
            benchmark.elapsed += delta;
            if let Some(mut transform) = world.player.iter_mut().next() {
                transform.translation.x += 8.0 * delta as f32;
            }
            if benchmark.elapsed >= 20.0 {
                let (p50, p95, p99) = diagnostics.frame_percentiles();
                info!("voxel benchmark movement: frame_ms[p50={p50:.2} p95={p95:.2} p99={p99:.2}]");
                benchmark.phase = BenchmarkPhase::Done;
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
    rendered_regions: Query<(), With<VoxelRenderRegion>>,
    pending: Res<PendingChunks>,
    render_regions: Res<RenderRegions>,
) {
    diagnostics.elapsed += time.delta_secs_f64();
    diagnostics
        .frames_ms
        .push_back(time.delta_secs_f64() * 1_000.0);
    if diagnostics.frames_ms.len() > MAX_FRAME_SAMPLES {
        diagnostics.frames_ms.pop_front();
    }
    if diagnostics.elapsed < 5.0 {
        return;
    }
    diagnostics.elapsed = 0.0;

    let loaded = chunks.iter().count();
    let building = builds.iter().count();
    let modified = chunks.iter().filter(|chunk| chunk.modified).count();
    let ready = loaded.saturating_sub(building);
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
        "voxel: loaded={loaded} ready={ready} pending={} building={building} render_regions={} dirty_regions={} modified={modified} frame_ms[p50={:.2} p95={:.2} p99={:.2}] generation_ms[p50={:.2} p95={:.2}] meshing_ms[p50={:.2} p95={:.2}] collider_ms[p50={:.2} p95={:.2}] generated={} built={} avg_vertices={} avg_triangles={}",
        pending.len(),
        rendered_regions.iter().count(),
        render_regions.dirty_len(),
        percentile(&frames, 0.50),
        percentile(&frames, 0.95),
        percentile(&frames, 0.99),
        percentile(&diagnostics.generation_ms, 0.50),
        percentile(&diagnostics.generation_ms, 0.95),
        percentile(&diagnostics.meshing_ms, 0.50),
        percentile(&diagnostics.meshing_ms, 0.95),
        percentile(&diagnostics.collider_ms, 0.50),
        percentile(&diagnostics.collider_ms, 0.95),
        diagnostics.generated_chunks,
        diagnostics.built_chunks,
        average_vertices,
        average_triangles,
    );

    diagnostics.generation_ms.clear();
    diagnostics.meshing_ms.clear();
    diagnostics.collider_ms.clear();
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

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn percentile_handles_unsorted_and_empty_samples() {
        assert_eq!(percentile(&[], 0.95), 0.0);
        assert_eq!(percentile(&[8.0, 2.0, 4.0, 6.0], 0.50), 6.0);
        assert_eq!(percentile(&[8.0, 2.0, 4.0, 6.0], 0.95), 8.0);
    }
}
