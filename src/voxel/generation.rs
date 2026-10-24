use super::{ChunkVoxels, VoxelKind, VoxelSettings};
use bevy::prelude::*;
use pumpkin_worldgen::{BlockInfo, GeneratedChunk, World, block_info};
use std::{
    collections::HashSet,
    sync::{Arc, OnceLock, mpsc},
};

pub(crate) const MIN_Y: i32 = -64;
const HEIGHT: usize = 384;
struct ChunkSource {
    cells: Vec<VoxelKind>,
    states: Arc<[u16]>,
    light: Arc<[u8]>,
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
            states: generated.block_states().into(),
            light: generated
                .light()
                .expect("full worldgen includes lighting")
                .into(),
        }
    }

    fn into_voxels(self, settings: &VoxelSettings) -> ChunkVoxels {
        assert_eq!(settings.chunk_size, 16, "worldgen requires 16x16 chunks");
        assert!((1..=384).contains(&settings.max_height));
        let cells = (256 * settings.max_height) as usize;
        let mut voxels = self.cells;
        voxels.truncate(cells);
        let light = self.light[..cells].into();
        let states = self.states[..cells].into();
        ChunkVoxels::generated(16, settings.max_height, voxels).with_lighting(states, light)
    }
}
impl std::ops::Deref for ChunkSource {
    type Target = Vec<VoxelKind>;
    fn deref(&self) -> &Self::Target {
        &self.cells
    }
}

/// Resolve aliases used by the game's full-cube material catalogue once.
pub(crate) fn native_state(kind: VoxelKind) -> u16 {
    static STATES: OnceLock<Box<[u16]>> = OnceLock::new();
    STATES.get_or_init(|| {
        VoxelKind::ALL
            .iter()
            .map(|kind| {
                let name = match kind {
                    VoxelKind::Grass => "grass_block",
                    VoxelKind::Wood => "oak_log",
                    VoxelKind::Leaves => "oak_leaves",
                    VoxelKind::Snow => "snow_block",
                    _ => kind.key(),
                };
                pumpkin_worldgen::default_block_state(name)
                    .expect("voxel material has a Minecraft block")
            })
            .collect()
    })[kind as usize]
}

/// Vanilla chunks from `pumpkin-worldgen` (terrain, caves, ores, trees, structures).
///
/// Streaming keeps Pumpkin's chunk system loaded around the viewer with one ticket,
/// so chunks sharing generation neighbors never regenerate or reload them.
#[derive(Resource, Clone)]
pub(crate) struct WorldGenerator {
    world: Arc<OnceLock<World>>,
    seed: u32,
    /// View changes go to a dedicated thread: retargeting Pumpkin's tickets recomputes
    /// thousands of chunk levels under a lock its scheduler also takes.
    views: mpsc::Sender<Option<(IVec2, u8)>>,
}

fn init_world(world: &OnceLock<World>, seed: u32) -> &World {
    world.get_or_init(|| World::new(i64::from(seed)))
}

/// Applies the latest requested view; older pending ones are skipped.
fn run_view_thread(
    world: Arc<OnceLock<World>>,
    seed: u32,
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
    pub(crate) fn new(seed: u32) -> Self {
        let world = Arc::<OnceLock<World>>::default();
        let (views, receiver) = mpsc::channel();
        let thread_world = world.clone();
        std::thread::Builder::new()
            .name("worldgen view".into())
            .spawn(move || run_view_thread(thread_world, seed, receiver))
            .expect("spawn worldgen view thread");
        Self { world, seed, views }
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

    pub(crate) fn is_loaded(&self, position: IVec2) -> bool {
        self.world().is_loaded(position.x, position.y)
    }

    /// Converts a chunk Pumpkin already holds; `None` once it was unloaded.
    pub(crate) fn loaded_chunk(
        &self,
        position: IVec2,
        settings: &VoxelSettings,
    ) -> Option<ChunkVoxels> {
        let generated = self.world().loaded_chunk(position.x, position.y)?;
        Some(ChunkSource::new(&generated).into_voxels(settings))
    }

    fn chunk(&self, position: IVec2) -> ChunkSource {
        let world = self.world();
        let generated = world
            .loaded_chunk(position.x, position.y)
            .unwrap_or_else(|| world.generate_chunk(position.x, position.y));
        ChunkSource::new(&generated)
    }

    pub(crate) fn spawn_position(&self) -> Vec3 {
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
                    let cells = self.chunk(IVec2::new(dx, dz));
                    let surface = top(&|i| cells[i]).unwrap_or(ground);
                    if surface + 2 >= HEIGHT {
                        continue;
                    }
                    return Vec3::new(
                        (dx * 16 + 8) as f32,
                        surface as f32 + 3.0,
                        (dz * 16 + 8) as f32,
                    );
                }
            }
        }
        Vec3::new(8.0, 200.0, 8.0)
    }
}

/// Blocks until the chunk exists; streaming uses [`WorldGenerator::loaded_chunk`] instead.
pub(crate) fn generate_chunk(
    position: IVec2,
    settings: &VoxelSettings,
    generator: &WorldGenerator,
) -> ChunkVoxels {
    generator.chunk(position).into_voxels(settings)
}

fn map_info(info: BlockInfo) -> VoxelKind {
    if info.is_air {
        return VoxelKind::Air;
    }
    if let Some(kind) = map_block(info.name) {
        return kind;
    }
    // Plants, torches, vines, fences... cannot be drawn as cubes: omit them.
    // Slabs and stairs stay solid so structures keep their floors and roofs.
    if info.hardness == 0.0
        || (info.opacity == 0 && !info.name.ends_with("_slab") && !info.name.ends_with("_stairs"))
    {
        // Coral, sea pickles, submerged fences... leave their water behind.
        return if info.is_waterlogged || info.is_liquid {
            VoxelKind::Water
        } else {
            VoxelKind::Air
        };
    }
    // Unrepresented solid materials remain solid instead of leaving holes.
    VoxelKind::Stone
}

/// Minecraft block name -> game block, independent of properties.
fn map_block(name: &str) -> Option<VoxelKind> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let name = name.strip_prefix("infested_").unwrap_or(name);
    let name = name.strip_prefix("stripped_").unwrap_or(name);
    let kind = match name {
        "air" | "cave_air" | "void_air" => VoxelKind::Air,
        "water" | "bubble_column" | "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant" => {
            VoxelKind::Water
        }
        "grass_block" => VoxelKind::Grass,
        "oak_log" | "oak_wood" => VoxelKind::Wood,
        "oak_leaves" => VoxelKind::Leaves,
        "suspicious_sand" => VoxelKind::Sand,
        "suspicious_gravel" => VoxelKind::Gravel,
        "chiseled_stone_bricks" => VoxelKind::StoneBricks,
        "cut_sandstone" | "chiseled_sandstone" | "smooth_sandstone" => VoxelKind::Sandstone,
        "cut_red_sandstone" | "chiseled_red_sandstone" | "smooth_red_sandstone" => {
            VoxelKind::RedSandstone
        }
        "packed_mud" | "mud_bricks" => VoxelKind::Mud,
        "carved_pumpkin" | "jack_o_lantern" => VoxelKind::Pumpkin,
        name if name.ends_with("_wood") => {
            return VoxelKind::from_key(&format!("{}_log", name.trim_end_matches("_wood")));
        }
        name if name.ends_with("_terracotta") && !name.contains("glazed") => {
            VoxelKind::from_key(name).unwrap_or(VoxelKind::Terracotta)
        }
        "smooth_stone" => VoxelKind::Stone,
        name => return VoxelKind::from_key(name).or_else(|| similar_block(name)),
    };
    Some(kind)
}

/// Blocks without their own kind -> closest material, so structures keep their look.
fn similar_block(name: &str) -> Option<VoxelKind> {
    // Shapes take their material: "stone_brick_stairs" -> "stone_bricks", "oak_slab" -> "oak_planks".
    for suffix in ["_slab", "_stairs", "_wall"] {
        if let Some(base) = name.strip_suffix(suffix) {
            return [
                base.to_owned(),
                format!("{base}s"),
                format!("{base}_planks"),
            ]
            .iter()
            .find_map(|base| map_block(base));
        }
    }
    for prefix in ["polished_", "chiseled_", "cracked_", "cut_", "smooth_"] {
        if let Some(kind) = name.strip_prefix(prefix).and_then(map_block) {
            return Some(kind);
        }
    }
    // Carpets are thin: left to `map_info`, which omits non-cube blocks.
    let dyed = ["_wool", "_concrete", "_concrete_powder"]
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix));
    let kind = match name {
        _ if dyed.is_some() => VoxelKind::from_key(&format!("{}_terracotta", dyed.unwrap()))
            .unwrap_or(VoxelKind::Terracotta),
        // Mangrove, cherry, pale oak, bamboo, crimson and warped planks.
        name if name.ends_with("_planks") || name == "bamboo_mosaic" => VoxelKind::OakPlanks,
        // Crimson and warped stems; crop stems (pumpkin, melon) are plants.
        "crimson_stem" | "warped_stem" | "crimson_hyphae" | "warped_hyphae" | "bamboo_block" => {
            VoxelKind::DarkOakLog
        }
        name if name.contains("deepslate") => VoxelKind::Deepslate,
        _ => return None,
    };
    Some(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_materials_preserve_air_water_and_solid_terrain() {
        assert_eq!(map_block("minecraft:cave_air"), Some(VoxelKind::Air));
        assert_eq!(map_block("water"), Some(VoxelKind::Water));
        assert_eq!(map_block("grass_block"), Some(VoxelKind::Grass));
        assert_eq!(map_block("stripped_birch_wood"), Some(VoxelKind::BirchLog));
        assert_eq!(map_block("bedrock"), Some(VoxelKind::Bedrock));
        assert_eq!(map_block("oak_fence"), None);
        assert_eq!(map_block("oak_stairs"), Some(VoxelKind::OakPlanks));
        assert_eq!(map_block("stone_brick_slab"), Some(VoxelKind::StoneBricks));
        assert_eq!(map_block("cherry_planks"), Some(VoxelKind::OakPlanks));
        assert_eq!(
            map_block("deepslate_tile_stairs"),
            Some(VoxelKind::Deepslate)
        );
        assert_eq!(map_block("lava"), Some(VoxelKind::Lava));
        assert_eq!(map_block("moss_carpet"), None);
        assert_eq!(map_block("white_carpet"), None);
        assert_eq!(map_block("pumpkin_stem"), None);
        assert_eq!(map_block("attached_melon_stem"), None);
        assert_eq!(map_block("warped_stem"), Some(VoxelKind::DarkOakLog));
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
        let chunk = generator.chunk(position);
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
