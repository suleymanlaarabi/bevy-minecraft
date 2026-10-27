use super::{ChunkVoxels, VoxelKind};
use bevy::prelude::*;
use hollow_core::coordinates::CHUNK_HEIGHT;
use hollow_worldgen::{Dimension, GenerationSession};
use std::sync::Arc;
pub(crate) const MIN_Y: i32 = -64;
const HEIGHT: usize = CHUNK_HEIGHT as usize;

/// Game storage and container materialization around the native generation session.
#[derive(Resource, Clone)]
pub(crate) struct WorldGenerator {
    pub(crate) runtime: GenerationSession,
    storage: Option<Arc<crate::world::storage::Database>>,
    errors: Arc<std::sync::Mutex<Vec<String>>>,
    blocks: hollow_engine::blocks::BlockCatalog,
}
fn index(x: usize, y: usize, z: usize) -> usize {
    x + 16 * (z + 16 * y)
}
pub(crate) enum ChunkLoad {
    Ready {
        voxels: ChunkVoxels,
        containers: crate::block_behavior::containers::PreparedColumn,
        revision: u64,
    },
    Retry,
    Missing,
}
impl WorldGenerator {
    pub(crate) fn new(seed: i64, blocks: hollow_engine::blocks::BlockCatalog) -> Self {
        Self {
            runtime: GenerationSession::new(
                seed,
                Dimension::Overworld,
                hollow_worldgen::TerrainQuality::Fast,
                None,
            ),
            storage: None,
            errors: Arc::default(),
            blocks,
        }
    }

    pub(crate) fn persistent(
        seed: i64,
        quality: crate::world::WorldQuality,
        storage: Arc<crate::world::storage::Database>,
        blocks: hollow_engine::blocks::BlockCatalog,
    ) -> Self {
        Self {
            runtime: GenerationSession::new(
                seed,
                Dimension::Overworld,
                quality.native(),
                Some(storage.clone()),
            ),
            storage: Some(storage),
            errors: Arc::default(),
            blocks,
        }
    }
    pub(crate) fn take_errors(&self) -> Vec<String> {
        let mut errors = std::mem::take(&mut *self.errors.lock().unwrap());
        errors.extend(self.runtime.take_errors());
        errors
    }
    pub(crate) async fn flush(&self) -> std::io::Result<()> {
        self.runtime.flush().await
    }
    pub(crate) fn clear_view(&self) {
        self.runtime.clear_view();
    }
    pub(crate) fn is_ready(&self, position: IVec2) -> bool {
        self.storage
            .as_ref()
            .is_some_and(|storage| storage.is_edited(position.x, position.y))
            || self.runtime.snapshot(position).is_some()
    }
    pub(crate) fn storage_revision(&self, position: IVec2) -> Option<u64> {
        self.storage.as_ref().map_or(Some(0), |database| {
            database.revision(position.x, position.y)
        })
    }
    pub(crate) fn cancel_requests(&self) {
        self.runtime.cancel();
    }
    /// Called on an I/O worker; await compute-only decoding after the WAL read.
    pub(crate) async fn loaded_gameplay_chunk(
        &self,
        position: IVec2,
        budget: hollow_worldgen::tasks::VoxelTaskBudget,
    ) -> ChunkLoad {
        let result = async {
            if self.runtime.is_cancelled() {
                return Ok(ChunkLoad::Missing);
            }
            let column = if let Some(storage) = &self.storage {
                storage.gameplay_column(position.x, position.y)?
            } else {
                Some(crate::world::storage::GameplayColumn::default())
            };
            let Some(column) = column else {
                return Ok(ChunkLoad::Retry);
            };
            if self.storage_revision(position) != Some(column.revision) {
                return Ok(ChunkLoad::Retry);
            }
            let generator = self.clone();
            budget
                .spawn(hollow_worldgen::tasks::WorkClass::Loading, 0, async move {
                    if generator.runtime.is_cancelled() {
                        return Ok(ChunkLoad::Missing);
                    }
                    let revision = column.revision;
                    let (edited, records) = column.decode()?;
                    let initial_ticks = generator
                        .runtime
                        .snapshot(position)
                        .map(|snapshot| (snapshot.min_y, snapshot.fluid_ticks));
                    let voxels = if let Some(bytes) = edited {
                        ChunkVoxels::decode_saved(&bytes)?
                    } else {
                        let Some(generated) = generator.runtime.snapshot(position) else {
                            return Ok(ChunkLoad::Missing);
                        };
                        ChunkVoxels::from_layers(generated.states, generated.light)
                    };
                    let mut containers =
                        crate::block_behavior::containers::PreparedColumn::discover(
                            &voxels,
                            position,
                            &generator.blocks,
                        );
                    if let Some(bytes) = records {
                        containers.restore(&bytes, position)?;
                        containers.revision = 1;
                    } else if let Some((min_y, ticks)) = initial_ticks {
                        for tick in ticks.iter() {
                            let native = IVec3::from_array(tick.position);
                            let local = IVec3::new(
                                native.x.rem_euclid(16),
                                native.y - min_y,
                                native.z.rem_euclid(16),
                            );
                            if let Some(cell) = hollow_core::coordinates::cell_index(local) {
                                let family = if pumpkin_data::fluid::Fluid::same_fluid_type(
                                    tick.fluid,
                                    pumpkin_data::fluid::Fluid::WATER.id,
                                ) {
                                    1
                                } else {
                                    2
                                };
                                containers.simulation.schedule(
                                    cell as u32,
                                    crate::simulation::TickChannel::Fluid,
                                    family,
                                    u64::from(tick.delay),
                                    tick.priority,
                                );
                            }
                        }
                    }
                    Ok::<_, std::io::Error>(ChunkLoad::Ready {
                        voxels,
                        containers,
                        revision,
                    })
                })
                .await
        }
        .await;
        match result {
            Ok(value) => value,
            Err(error) => {
                self.errors
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(format!(
                        "Cannot load gameplay column ({}, {}): {error}",
                        position.x, position.y
                    ));
                ChunkLoad::Missing
            }
        }
    }

    pub(crate) async fn try_spawn_position(
        &self,
        budget: hollow_worldgen::tasks::VoxelTaskBudget,
    ) -> std::io::Result<Vec3> {
        self.runtime.ready().await?;
        // Prefer dry ground near the origin, which can be ocean.
        // Candidates are probed on terrain-only chunks, far cheaper than full ones.
        let top = |cells: &dyn Fn(usize) -> VoxelKind| {
            (0..HEIGHT)
                .rev()
                .find(|&y| cells(index(8, y, 8)) != VoxelKind::Air)
        };
        for radius in 0i32..=16 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if self.runtime.is_cancelled() {
                        return Err(std::io::Error::other("World request cancelled"));
                    }
                    if dx.abs().max(dz.abs()) != radius {
                        continue;
                    }
                    let runtime = self.runtime.clone();
                    let terrain = budget
                        .spawn(hollow_worldgen::tasks::WorkClass::Loading, 0, async move {
                            runtime.probe_terrain(IVec2::new(dx, dz))
                        })
                        .await;
                    let terrain = |i: usize| terrain.states.get(i).kind();
                    let Some(ground) = top(&terrain) else {
                        continue;
                    };
                    // Sea level is Minecraft Y=63.
                    if (ground as i32) < 63 - MIN_Y || !terrain(index(8, ground, 8)).is_solid() {
                        continue;
                    }
                    // Hold the chosen base until the loading viewer takes over its demand.
                    self.runtime.set_view(IVec2::new(dx, dz), 0);
                    // Trees only exist in the full chunk.
                    let cells = self.runtime.request_chunk(IVec2::new(dx, dz)).await?;
                    let surface = top(&|i| cells.states.get(i).kind()).unwrap_or(ground);
                    if surface + 2 >= HEIGHT {
                        continue;
                    }
                    return Ok(Vec3::new(
                        (dx * 16 + 8) as f32,
                        surface as f32 + 3.0,
                        (dz * 16 + 8) as f32,
                    ));
                }
            }
        }
        Ok(Vec3::new(8.0, 200.0, 8.0))
    }
}
