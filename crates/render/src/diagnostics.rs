use bevy::prelude::*;
use std::{collections::VecDeque, time::Duration};
#[derive(Resource, Default)]
pub struct VoxelDiagnostics {
    pub frames_ms: VecDeque<f64>,
    pub generation_ms: Vec<f64>,
    pub meshing_ms: Vec<f64>,
    pub generated_chunks: u64,
    pub built_chunks: u64,
    pub built_vertices: u64,
    pub built_triangles: u64,
    pub elapsed: f64,
}

impl VoxelDiagnostics {
    pub fn record_generation(&mut self, elapsed: Duration) {
        self.generated_chunks += 1;
        self.generation_ms.push(elapsed.as_secs_f64() * 1_000.0);
    }

    pub fn record_build(&mut self, meshing: Duration, vertices: usize, triangles: usize) {
        self.built_chunks += 1;
        self.built_vertices += vertices as u64;
        self.built_triangles += triangles as u64;
        self.meshing_ms.push(meshing.as_secs_f64() * 1_000.0);
    }

    pub fn reset_frames(&mut self) {
        self.frames_ms.clear();
    }

    pub fn frame_percentiles(&self) -> (f64, f64, f64) {
        let frames: Vec<_> = self.frames_ms.iter().copied().collect();
        (
            percentile(&frames, 0.50),
            percentile(&frames, 0.95),
            percentile(&frames, 0.99),
        )
    }
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
