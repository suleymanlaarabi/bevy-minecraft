use super::{ChunkVoxels, VoxelKind, coordinates::CHUNK_HEIGHT};
use bevy::prelude::*;
use pumpkin_worldgen::{BlockInfo, GeneratedChunk, World, block_info};
use std::{
    collections::HashSet,
    sync::{Arc, OnceLock, mpsc},
};

pub(crate) const MIN_Y: i32 = -64;
const HEIGHT: usize = CHUNK_HEIGHT as usize;
struct ChunkSource {
    cells: Vec<VoxelKind>,
}
impl ChunkSource {
    fn new(generated: &GeneratedChunk) -> Self {
        let kinds = kinds();
        Self {
            cells: generated
                .block_states()
                .iter()
                .map(|&state| kinds[state as usize])
                .collect(),
        }
    }
}
impl std::ops::Deref for ChunkSource {
    type Target = Vec<VoxelKind>;
    fn deref(&self) -> &Self::Target {
        &self.cells
    }
}

/// Vanilla chunks from `pumpkin-worldgen` (terrain, caves, ores, trees, structures).
///
/// Streaming keeps Pumpkin's chunk system loaded around the viewer with one ticket,
/// so chunks sharing generation neighbors never regenerate or reload them.
#[derive(Resource, Clone)]
pub(crate) struct WorldGenerator {
    world: Arc<OnceLock<World>>,
    seed: i64,
    /// View changes go to a dedicated thread: retargeting Pumpkin's tickets recomputes
    /// thousands of chunk levels under a lock its scheduler also takes.
    views: mpsc::Sender<Option<(IVec2, u8)>>,
    storage: Option<Arc<crate::world::storage::Database>>,
    errors: Arc<std::sync::Mutex<Vec<String>>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

fn init_world(world: &OnceLock<World>, seed: i64) -> &World {
    world.get_or_init(|| {
        World::with_options(
            seed,
            pumpkin_worldgen::Dimension::Overworld,
            generation_options(),
        )
    })
}

fn generation_options() -> pumpkin_worldgen::WorldOptions {
    let cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    pumpkin_worldgen::WorldOptions {
        generation_threads: Some((cores / 4).clamp(1, 6)),
        ..default()
    }
}

/// Applies the latest requested view; older pending ones are skipped.
fn run_view_thread(
    world: Arc<OnceLock<World>>,
    seed: i64,
    views: mpsc::Receiver<Option<(IVec2, u8)>>,
) {
    while let Ok(mut view) = views.recv() {
        view = views.try_iter().last().unwrap_or(view);
        match view {
            Some((center, radius)) => init_world(&world, seed).set_view(center.x, center.y, radius),
            None => {
                if let Some(world) = world.get() {
                    world.clear_view();
                }
            }
        }
    }
}

/// Same Y-major layout as `GeneratedChunk::block_states`.
fn index(x: usize, y: usize, z: usize) -> usize {
    x + 16 * (z + 16 * y)
}

/// Block-state id -> game material, computed once for every vanilla state.
fn kinds() -> &'static [VoxelKind] {
    static KINDS: OnceLock<Box<[VoxelKind]>> = OnceLock::new();
    KINDS.get_or_init(|| {
        let mut approximated = HashSet::new();
        (0..=u16::MAX)
            .map(|id| {
                let Some(info) = block_info(id) else {
                    return VoxelKind::Air;
                };
                let kind = map_info(info);
                if kind == VoxelKind::Stone
                    && map_block(info.name).is_none()
                    && approximated.insert(info.name)
                {
                    debug!("No voxel kind for {}: drawn as stone", info.name);
                }
                kind
            })
            .collect()
    })
}

impl WorldGenerator {
    pub(crate) fn new(seed: i64) -> Self {
        let world = Arc::<OnceLock<World>>::default();
        let (views, receiver) = mpsc::channel();
        let thread_world = world.clone();
        std::thread::Builder::new()
            .name("worldgen view".into())
            .spawn(move || run_view_thread(thread_world, seed, receiver))
            .expect("spawn worldgen view thread");
        Self {
            world,
            seed,
            views,
            storage: None,
            errors: Arc::default(),
            cancelled: Arc::default(),
        }
    }

    pub(crate) fn persistent(
        seed: i64,
        quality: crate::world::WorldQuality,
        storage: Arc<crate::world::storage::Database>,
    ) -> Self {
        let world = Arc::new(OnceLock::new());
        let options = pumpkin_worldgen::WorldOptions {
            quality: quality.native(),
            ..generation_options()
        };
        let _ = world.set(World::with_storage(
            seed,
            pumpkin_worldgen::Dimension::Overworld,
            options,
            storage.clone(),
        ));
        let (views, receiver) = mpsc::channel();
        let thread_world = world.clone();
        std::thread::Builder::new()
            .name("worldgen view".into())
            .spawn(move || run_view_thread(thread_world, seed, receiver))
            .expect("spawn worldgen view thread");
        Self {
            world,
            seed,
            views,
            storage: Some(storage),
            errors: Arc::default(),
            cancelled: Arc::default(),
        }
    }

    pub(crate) fn take_errors(&self) -> Vec<String> {
        let mut errors = std::mem::take(
            &mut *self
                .errors
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if let Some(world) = self.world.get() {
            errors.extend(world.take_storage_errors());
        }
        errors
    }

    pub(crate) fn flush(&self) -> std::io::Result<()> {
        if let Some(world) = self.world.get() {
            world.flush_storage()?;
        }
        Ok(())
    }

    fn world(&self) -> &World {
        init_world(&self.world, self.seed)
    }

    /// Keeps every chunk within `radius` of `center` generated (square area), without blocking.
    pub(crate) fn set_view(&self, center: IVec2, radius: u32) {
        let radius = radius.min(u32::from(u8::MAX)) as u8;
        let _ = self.views.send(Some((center, radius)));
    }

    pub(crate) fn clear_view(&self) {
        let _ = self.views.send(None);
    }

    /// Chunks finished since the last call.
    pub(crate) fn finished_chunks(&self) -> impl Iterator<Item = IVec2> + '_ {
        self.world()
            .finished_chunks()
            .map(|(x, z)| IVec2::new(x, z))
    }

    /// Whether `loaded_chunk` can return data now: a saved snapshot never waits for Pumpkin.
    pub(crate) fn is_ready(&self, position: IVec2) -> bool {
        self.storage
            .as_ref()
            .is_some_and(|storage| storage.is_edited(position.x, position.y))
            || self.world().is_loaded(position.x, position.y)
    }

    /// Reads a saved snapshot, else converts a chunk Pumpkin already holds;
    /// `None` once it was unloaded.
    pub(crate) fn loaded_chunk(&self, position: IVec2) -> Option<ChunkVoxels> {
        if let Some(storage) = &self.storage {
            match storage
                .edited(position.x, position.y)
                .and_then(|bytes| bytes.map(|b| ChunkVoxels::decode_saved(&b)).transpose())
            {
                Ok(Some(chunk)) => return Some(chunk),
                Ok(None) => {}
                Err(error) => {
                    self.errors
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(error.to_string());
                    return None;
                }
            }
        }
        let generated = self.world().loaded_chunk(position.x, position.y)?;
        Some(ChunkVoxels::from_native(
            generated.block_states(),
            generated.light().expect("full worldgen includes lighting"),
            kinds(),
        ))
    }

    fn chunk(&self, position: IVec2) -> std::io::Result<ChunkSource> {
        let world = self.world();
        let generated = world
            .loaded_chunk(position.x, position.y)
            .map_or_else(|| world.try_generate_chunk(position.x, position.y), Ok)?;
        Ok(ChunkSource::new(&generated))
    }

    #[cfg(test)]
    fn spawn_position(&self) -> Vec3 {
        self.try_spawn_position().expect("spawn position")
    }

    pub(crate) fn cancel_requests(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(world) = self.world.get() {
            world.cancel_requests();
        }
    }

    pub(crate) fn try_spawn_position(&self) -> std::io::Result<Vec3> {
        // Prefer dry ground near the origin, which can be ocean.
        // Candidates are probed on terrain-only chunks, far cheaper than full ones.
        let kinds = kinds();
        let top = |cells: &dyn Fn(usize) -> VoxelKind| {
            (0..HEIGHT)
                .rev()
                .find(|&y| cells(index(8, y, 8)) != VoxelKind::Air)
        };
        for radius in 0i32..=16 {
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    if self.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err(std::io::Error::other("World request cancelled"));
                    }
                    if dx.abs().max(dz.abs()) != radius {
                        continue;
                    }
                    let terrain = self.world().generate_terrain(dx, dz);
                    let terrain = |i: usize| kinds[terrain.block_states()[i] as usize];
                    let Some(ground) = top(&terrain) else {
                        continue;
                    };
                    // Sea level is Minecraft Y=63.
                    if (ground as i32) < 63 - MIN_Y || !terrain(index(8, ground, 8)).is_solid() {
                        continue;
                    }
                    // Trees only exist in the full chunk.
                    let cells = self.chunk(IVec2::new(dx, dz))?;
                    let surface = top(&|i| cells[i]).unwrap_or(ground);
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

fn map_info(info: BlockInfo) -> VoxelKind {
    if info.is_air {
        VoxelKind::Air
    } else {
        VoxelKind::from_key(info.name).unwrap_or(VoxelKind::Stone)
    }
}

fn map_block(name: &str) -> Option<VoxelKind> {
    VoxelKind::from_key(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_materials_preserve_air_water_and_solid_terrain() {
        for name in [
            "cave_air",
            "water",
            "grass_block",
            "stripped_birch_wood",
            "bedrock",
            "oak_fence",
            "oak_stairs",
            "stone_brick_slab",
            "cherry_planks",
            "deepslate_tile_stairs",
            "lava",
            "moss_carpet",
            "white_carpet",
            "pumpkin_stem",
            "attached_melon_stem",
            "warped_stem",
            "glass",
            "short_grass",
            "pointed_dripstone",
        ] {
            let native = map_block(name).expect("native block");
            assert_eq!(native.name(), name);
        }
        assert_eq!(map_block("minecraft:grass_block"), Some(VoxelKind::Grass));
        let kinds = kinds();
        assert!(kinds.contains(&VoxelKind::Leaves) && kinds.contains(&VoxelKind::Water));
    }

    #[test]
    fn generated_chunks_have_ores_and_a_dry_spawn() {
        let generator = WorldGenerator::new(42);
        let spawn = generator.spawn_position();
        assert!(spawn.y > 64.0);
        let position = IVec2::new(
            (spawn.x as i32).div_euclid(16),
            (spawn.z as i32).div_euclid(16),
        );
        let chunk = generator.chunk(position).unwrap();
        let x = (spawn.x as i32).rem_euclid(16) as usize;
        let z = (spawn.z as i32).rem_euclid(16) as usize;
        assert_ne!(chunk[index(x, spawn.y as usize - 3, z)], VoxelKind::Air);
        assert_eq!(chunk[index(x, spawn.y as usize - 2, z)], VoxelKind::Air);
        assert!(chunk.iter().any(|&kind| {
            matches!(
                kind,
                VoxelKind::CoalOre
                    | VoxelKind::IronOre
                    | VoxelKind::CopperOre
                    | VoxelKind::DiamondOre
            )
        }));
    }
}
