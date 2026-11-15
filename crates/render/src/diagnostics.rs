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
    pub load_wall_seconds: f64,
    pub mesh_worker_seconds: f64,
    pub mesh_apply_seconds: f64,
    pub visibility_seconds: f64,
    pub visibility_columns: u64,
    pub visibility_remaining: usize,
    pub visibility_conservative: bool,
    pub translucent_sort_seconds: f64,
    // Asset bytes queued on the main world, before render extraction/GPU upload.
    pub uploaded_bytes: u64,
    pub uploaded_meshes: u64,
    pub elapsed: f64,
}

impl VoxelDiagnostics {
    pub fn record_generation(&mut self, elapsed: Duration) {
        self.generated_chunks += 1;
        self.load_wall_seconds += elapsed.as_secs_f64();
        self.generation_ms.push(elapsed.as_secs_f64() * 1_000.0);
    }

    pub fn record_build(&mut self, meshing: Duration, vertices: usize, triangles: usize) {
        self.built_chunks += 1;
        self.mesh_worker_seconds += meshing.as_secs_f64();
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

#[cfg(feature = "dev")]
pub mod memory {
    use bevy::{
        ecs::{
            change_detection::{MaybeLocation, Tick},
            component::StorageType,
        },
        prelude::*,
    };
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
        time::Instant,
    };

    #[derive(Clone, Copy, Default, Debug)]
    pub struct EcsMemory {
        pub archetypes: usize,
        pub tables: usize,
        pub table_live: usize,
        pub table_reserved: usize,
        pub sparse_live_lower_bound: usize,
        pub largest_unused_table: usize,
    }
    pub fn ecs_memory(world: &World) -> EcsMemory {
        let mut result = EcsMemory {
            archetypes: world.archetypes().len(),
            ..Default::default()
        };
        let mut seen = HashSet::new();
        let ticks = 2 * size_of::<Tick>()
            + size_of::<MaybeLocation<&'static std::panic::Location<'static>>>();
        for archetype in world.archetypes().iter() {
            let id = archetype.table_id();
            if !seen.insert(id.as_usize()) {
                continue;
            }
            let table = &world.storages().tables[id];
            let row = size_of::<Entity>()
                + archetype
                    .table_components()
                    .map(|id| world.components().get_info(id).unwrap().layout().size() + ticks)
                    .sum::<usize>();
            let live = table.entities().len() * row;
            let reserved = table.capacity() * row;
            result.tables += 1;
            result.table_live += live;
            result.table_reserved += reserved;
            result.largest_unused_table = result.largest_unused_table.max(reserved - live);
        }
        for (id, sparse) in world.storages().sparse_sets.iter() {
            let info = world.components().get_info(id).unwrap();
            debug_assert_eq!(info.storage_type(), StorageType::SparseSet);
            // Bevy exposes sparse population, but not its column/index capacities.
            result.sparse_live_lower_bound +=
                sparse.len() * (info.layout().size() + ticks + size_of::<Entity>());
        }
        result
    }

    #[derive(Clone, Default, Debug)]
    pub struct RenderMemory {
        pub ecs: EcsMemory,
        pub cpu_payload: usize,
        pub arena_live: u64,
        pub arena_reserved: u64,
    }
    #[derive(Resource, Clone, Default)]
    pub struct MemoryProbe(pub Arc<Mutex<RenderMemory>>);
    impl MemoryProbe {
        pub fn enabled() -> bool {
            static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            *ENABLED.get_or_init(|| {
                std::env::var_os("HOLLOW_DIAGNOSTICS").is_some()
                    || std::env::var_os("HOLLOW_FLIGHT_BENCH").is_some()
            })
        }
    }
    pub fn queued_payload_bytes(world: &World) -> usize {
        world
            .get_resource::<crate::sections::SectionFeed>()
            .map_or(0, crate::sections::SectionFeed::retained_bytes)
    }
    pub fn sample_due(last: &mut Option<Instant>) -> bool {
        if last.is_some_and(|time| time.elapsed().as_secs_f64() < 1.0) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }
}
