use super::{ChunkVoxels, VoxelKind, VoxelSettings};
use bevy::prelude::*;
use mc_worldgen::{
    ChunkPos, Dimension,
    decoration::{FeatureOrder, vanilla_biome_features},
    decoration_world::{DecorationPass, DecorationRegion},
    density::beardifier::Beardifier,
    features::{BlockCatalog, FeatureProgram, PlacedFeature},
    random::{LegacyRandomSource, RandomSource},
    world::{WorldTerrainChunk, WorldTerrainGenerator},
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, OnceLock},
};

pub(crate) const MIN_Y: i32 = -64;
const HEIGHT: usize = 384;
const CACHE_SIZE: usize = 16;
type Patch = Vec<(IVec3, VoxelKind)>;

#[derive(Resource, Clone)]
pub(crate) struct WorldGenerator {
    seed: u32,
    shared_chunks: Arc<Mutex<VecDeque<(IVec2, Arc<Vec<VoxelKind>>)>>>,
}

struct Worker {
    seed: u32,
    generator: WorldTerrainGenerator,
    order: FeatureOrder,
    supported: HashMap<String, bool>,
    terrain: VecDeque<(IVec2, Arc<WorldTerrainChunk>)>,
    patches: VecDeque<(IVec2, Arc<Patch>)>,
    chunks: VecDeque<(IVec2, Arc<Vec<VoxelKind>>)>,
}

thread_local! {
    static WORKER: RefCell<Option<Worker>> = const { RefCell::new(None) };
}

fn is_tree_feature(key: &str) -> bool {
    key.contains("trees_") || key.ends_with(":dark_forest_vegetation")
}

fn report_omitted(key: &str, error: impl std::fmt::Display) {
    static REPORTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    if REPORTED
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(key.to_owned())
    {
        warn!("Partial world decoration: {key}: {error}");
    }
}

fn cached<T>(cache: &mut VecDeque<(IVec2, Arc<T>)>, key: IVec2) -> Option<Arc<T>> {
    let index = cache.iter().position(|(position, _)| *position == key)?;
    let entry = cache.remove(index).unwrap();
    let result = entry.1.clone();
    cache.push_back(entry);
    Some(result)
}
fn insert<T>(cache: &mut VecDeque<(IVec2, Arc<T>)>, key: IVec2, value: Arc<T>, limit: usize) {
    cache.push_back((key, value));
    if cache.len() > limit {
        cache.pop_front();
    }
}
fn index(x: usize, y: usize, z: usize) -> usize {
    x + 16 * (z + 16 * y)
}

impl Worker {
    fn terrain(&mut self, position: IVec2) -> Arc<WorldTerrainChunk> {
        if let Some(chunk) = cached(&mut self.terrain, position) {
            return chunk;
        }
        let chunk = Arc::new(
            self.generator
                .generate(
                    ChunkPos::new(position.x, position.y),
                    Beardifier::new(vec![], vec![], None),
                )
                .expect("minegen Overworld terrain generation failed"),
        );
        self.generator.biomes().clear_chunks();
        insert(&mut self.terrain, position, chunk.clone(), 48);
        chunk
    }

    fn region(&mut self, center: IVec2) -> DecorationRegion {
        let mut chunks = Vec::with_capacity(9);
        for dx in -1..=1 {
            for dz in -1..=1 {
                let chunk = self.terrain(center + IVec2::new(dx, dz));
                chunks.push(WorldTerrainChunk {
                    dimension: chunk.dimension,
                    terrain: chunk.terrain.clone(),
                    biomes: chunk.biomes.clone(),
                });
            }
        }
        DecorationRegion::from_terrain(
            i64::from(self.seed),
            ChunkPos::new(center.x, center.y),
            chunks,
        )
        .expect("valid Overworld decoration region")
    }

    fn supported(&mut self, key: &str) -> bool {
        *self
            .supported
            .entry(key.to_owned())
            .or_insert_with(|| match PlacedFeature::vanilla(key) {
                Ok(_) => true,
                Err(error) => {
                    report_omitted(key, error);
                    false
                }
            })
    }

    /// Each source chunk decorates an isolated 3x3 terrain region. Retain ALL its
    /// writes, including neighboring trunks/leaves, as a deterministic contribution.
    /// Targets combine the same nine contributions in X/Z order on every worker.
    /// This is a gameplay composition, not Vanilla's full decoration scheduler.
    fn patch(&mut self, center: IVec2) -> Arc<Patch> {
        if let Some(patch) = cached(&mut self.patches, center) {
            return patch;
        }
        let position = ChunkPos::new(center.x, center.y);
        let mut region = self.region(center);
        let mut ids = std::collections::BTreeSet::new();
        for chunk in region.chunks().unwrap() {
            ids.extend(chunk.biomes.biomes().iter().cloned());
        }
        let nearby = ids
            .iter()
            .map(|id| vanilla_biome_features(id).unwrap())
            .collect::<Vec<_>>();
        let mut plan = self
            .order
            .plan(i64::from(self.seed), position, &nearby, &[])
            .unwrap();
        // Vegetation patches/root systems and lighting-dependent plants are not
        // representable by the current cube renderer. Keep supported trees/bamboo.
        plan.calls.retain(|call| {
            [1, 2, 6, 8].contains(&call.step)
                || (call.step == 9 && (is_tree_feature(&call.key) || call.key.ends_with(":bamboo")))
        });
        let mut missing_trees = false;
        plan.calls.retain(|call| {
            let supported = self.supported(&call.key);
            missing_trees |= !supported && is_tree_feature(&call.key);
            supported
        });
        let mut terrain_plan = plan.clone();
        terrain_plan.calls.retain(|call| call.step != 9);
        let pass = DecorationPass::compile(position, plan).unwrap();
        if let Err(error) = region.execute(&pass) {
            // Never read the poisoned partial region after a failed optional pass.
            report_omitted("vegetation runtime", error);
            region = self.region(center);
            let pass = DecorationPass::compile(position, terrain_plan.clone()).unwrap();
            if let Err(error) = region.execute(&pass) {
                report_omitted("terrain decoration runtime", error);
                region = self.region(center);
            }
            missing_trees = true;
        }
        if missing_trees && let Err(error) = self.fallback_trees(&mut region, center) {
            report_omitted("fallback tree runtime", error);
            region = self.region(center);
            let pass = DecorationPass::compile(position, terrain_plan).unwrap();
            if let Err(error) = region.execute(&pass) {
                report_omitted("terrain decoration runtime", error);
                region = self.region(center);
            }
        }
        let mut changes = Vec::new();
        for chunk in region.chunks().unwrap() {
            let p = chunk.position();
            let before = self.terrain(IVec2::new(p.x, p.z));
            let palette = chunk.palette().iter().map(map_state).collect::<Vec<_>>();
            let old_palette = before.terrain.palette().map(map_state).collect::<Vec<_>>();
            for (i, &state) in chunk.blocks().iter().enumerate() {
                let kind = palette[state as usize];
                if kind != old_palette[before.terrain.blocks()[i] as usize] {
                    let y = i % HEIGHT;
                    let column = i / HEIGHT;
                    changes.push((
                        IVec3::new(
                            p.x * 16 + (column % 16) as i32,
                            y as i32,
                            p.z * 16 + (column / 16) as i32,
                        ),
                        kind,
                    ));
                }
            }
        }
        let changes = Arc::new(changes);
        insert(&mut self.patches, center, changes.clone(), 32);
        changes
    }

    fn fallback_trees(
        &mut self,
        region: &mut DecorationRegion,
        center: IVec2,
    ) -> Result<(), mc_worldgen::features::FeatureError> {
        // When a biome selector still contains an unsupported decorator (e.g.
        // beehives), use a supported minegen tree, with explicit game placement.
        let biome = region
            .biome_at([center.x * 16 + 8, 80, center.y * 16 + 8])
            .unwrap();
        let (tree, attempts) = if biome.contains("taiga") || biome.contains("grove") {
            ("spruce", 5)
        } else if biome.contains("birch") {
            ("birch", 5)
        } else if biome.contains("forest") {
            ("oak", 5)
        } else if biome.contains("savanna") {
            ("acacia", 2)
        } else if biome.contains("plains") {
            ("oak", 1)
        } else {
            return Ok(());
        };
        let feature = FeatureProgram::vanilla(&format!("minecraft:{tree}"))?;
        let mut random = LegacyRandomSource::new(
            i64::from(self.seed)
                ^ i64::from(center.x).wrapping_mul(341873128712)
                ^ i64::from(center.y).wrapping_mul(132897987541),
        );
        for _ in 0..attempts {
            let x = random.next_int_bound(16).unwrap();
            let z = random.next_int_bound(16).unwrap();
            let chunk = region.chunk(ChunkPos::new(center.x, center.y)).unwrap();
            let y = chunk.heights("OCEAN_FLOOR").unwrap()[(x + z * 16) as usize];
            if y <= 63
                || chunk.block(x as u8, y - 1, z as u8).is_none_or(|s| {
                    !matches!(
                        s.name.as_str(),
                        "minecraft:grass_block"
                            | "minecraft:dirt"
                            | "minecraft:podzol"
                            | "minecraft:snow_block"
                    )
                })
            {
                continue;
            }
            feature.place(
                region,
                &mut random,
                [center.x * 16 + x, y, center.y * 16 + z],
            )?;
        }
        Ok(())
    }

    fn chunk(&mut self, position: IVec2) -> Arc<Vec<VoxelKind>> {
        if let Some(chunk) = cached(&mut self.chunks, position) {
            return chunk;
        }
        let base = self.terrain(position);
        let palette = base.terrain.palette().map(map_state).collect::<Vec<_>>();
        let mut cells = vec![VoxelKind::Air; 256 * HEIGHT];
        for z in 0..16 {
            for x in 0..16 {
                for y in 0..HEIGHT {
                    cells[index(x, y, z)] =
                        palette[base.terrain.blocks()[y + (x + z * 16) * HEIGHT] as usize];
                }
            }
        }
        for dx in -1..=1 {
            for dz in -1..=1 {
                let patch = self.patch(position + IVec2::new(dx, dz));
                for &(world, kind) in patch.iter() {
                    let x = world.x - position.x * 16;
                    let z = world.z - position.y * 16;
                    if (0..16).contains(&x) && (0..16).contains(&z) {
                        cells[index(x as usize, world.y as usize, z as usize)] = kind;
                    }
                }
            }
        }
        let chunk = Arc::new(cells);
        insert(&mut self.chunks, position, chunk.clone(), CACHE_SIZE);
        chunk
    }
}

impl WorldGenerator {
    pub(crate) fn new(seed: u32) -> Self {
        Self {
            seed,
            shared_chunks: default(),
        }
    }
    fn chunk(&self, position: IVec2) -> Arc<Vec<VoxelKind>> {
        if let Some(chunk) = cached(&mut self.shared_chunks.lock().unwrap(), position) {
            return chunk;
        }
        let chunk = self.with_worker(|worker| worker.chunk(position));
        let mut shared = self.shared_chunks.lock().unwrap();
        if let Some(existing) = cached(&mut shared, position) {
            return existing;
        }
        insert(&mut shared, position, chunk.clone(), 64);
        chunk
    }
    fn with_worker<R>(&self, f: impl FnOnce(&mut Worker) -> R) -> R {
        WORKER.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_none_or(|w| w.seed != self.seed) {
                let mut generator =
                    WorldTerrainGenerator::vanilla(i64::from(self.seed), Dimension::Overworld)
                        .unwrap();
                let order = FeatureOrder::vanilla(&generator.biomes().possible_biomes()).unwrap();
                *slot = Some(Worker {
                    seed: self.seed,
                    generator,
                    order,
                    supported: HashMap::new(),
                    terrain: VecDeque::new(),
                    patches: VecDeque::new(),
                    chunks: VecDeque::new(),
                });
            }
            f(slot.as_mut().unwrap())
        })
    }
    pub(crate) fn spawn_position(&self) -> Vec3 {
        self.with_worker(|worker| {
            // Prefer dry ground over the old fixed origin, which can be ocean.
            for radius in 0i32..=16 {
                for dx in -radius..=radius {
                    for dz in -radius..=radius {
                        if dx.abs().max(dz.abs()) != radius {
                            continue;
                        }
                        let position = IVec2::new(dx, dz);
                        let chunk = worker.terrain(position);
                        let floor = chunk.terrain.ocean_floor_height(8, 8).unwrap();
                        let surface = chunk.terrain.world_surface_height(8, 8).unwrap();
                        if floor < 64 || floor != surface {
                            continue;
                        }
                        if !chunk.terrain.block(8, floor - 1, 8).is_some_and(|s| {
                            matches!(
                                s.name.as_str(),
                                "minecraft:grass_block" | "minecraft:sand" | "minecraft:snow_block"
                            )
                        }) {
                            continue;
                        }
                        let cells = worker.chunk(position);
                        let y = (floor - MIN_Y) as usize;
                        if cells[index(8, y, 8)] != VoxelKind::Air
                            || cells[index(8, y + 1, 8)] != VoxelKind::Air
                        {
                            continue;
                        }
                        insert(&mut self.shared_chunks.lock().unwrap(), position, cells, 64);
                        return Vec3::new(
                            (dx * 16 + 8) as f32,
                            y as f32 + 2.0,
                            (dz * 16 + 8) as f32,
                        );
                    }
                }
            }
            Vec3::new(8.0, 200.0, 8.0)
        })
    }
}

pub(crate) fn generate_chunk(
    position: IVec2,
    settings: &VoxelSettings,
    generator: &WorldGenerator,
) -> ChunkVoxels {
    assert_eq!(settings.chunk_size, 16, "minegen requires 16x16 chunks");
    assert!((1..=384).contains(&settings.max_height));
    let center = generator.chunk(position);
    // Do not generate four extra decorated chunks before showing the first one.
    // Existing samples are reused; streaming fills both borders as neighbors arrive.
    let neighbors = [IVec2::NEG_X, IVec2::X, IVec2::NEG_Y, IVec2::Y].map(|direction| {
        cached(
            &mut generator.shared_chunks.lock().unwrap(),
            position + direction,
        )
    });
    let mut halo = Vec::with_capacity((64 * settings.max_height) as usize);
    for y in 0..settings.max_height as usize {
        for (side, neighbor) in neighbors.iter().enumerate() {
            for offset in 0..16 {
                let (x, z) = match side {
                    0 => (15, offset),
                    1 => (0, offset),
                    2 => (offset, 15),
                    _ => (offset, 0),
                };
                halo.push(
                    neighbor
                        .as_ref()
                        .map_or(VoxelKind::Air, |cells| cells[index(x, y, z)]),
                );
            }
        }
    }
    ChunkVoxels::generated(
        16,
        settings.max_height,
        center[..(256 * settings.max_height) as usize].to_vec(),
        halo,
    )
}

fn map_state(state: &mc_worldgen::BlockState) -> VoxelKind {
    let kind = map_block(&state.name);
    if kind == VoxelKind::Stone
        && let Ok(flags) = BlockCatalog::flags(state)
        && !flags.solid()
    {
        return match flags.fluid() {
            "minecraft:water" | "minecraft:flowing_water" => VoxelKind::Water,
            "minecraft:empty" => VoxelKind::Air,
            _ => kind,
        };
    }
    kind
}

/// Approximation for the game's small block registry, independent of properties.
/// Unrepresented solid materials remain solid instead of leaving holes.
fn map_block(name: &str) -> VoxelKind {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    match name {
        "air" | "cave_air" | "void_air" => VoxelKind::Air,
        "water" => VoxelKind::Water,
        "deepslate" => VoxelKind::Deepslate,
        "gravel" => VoxelKind::Gravel,
        "coal_ore" | "deepslate_coal_ore" => VoxelKind::CoalOre,
        "iron_ore" | "deepslate_iron_ore" => VoxelKind::IronOre,
        "copper_ore" | "deepslate_copper_ore" => VoxelKind::CopperOre,
        "gold_ore" | "deepslate_gold_ore" => VoxelKind::GoldOre,
        "redstone_ore" | "deepslate_redstone_ore" => VoxelKind::RedstoneOre,
        "diamond_ore" | "deepslate_diamond_ore" => VoxelKind::DiamondOre,
        "emerald_ore" | "deepslate_emerald_ore" => VoxelKind::EmeraldOre,
        "lapis_ore" | "deepslate_lapis_ore" => VoxelKind::LapisOre,

        "grass_block" | "mycelium" => VoxelKind::Grass,
        "dirt" | "coarse_dirt" | "rooted_dirt" | "podzol" | "mud" => VoxelKind::Dirt,
        "sand" | "red_sand" | "sandstone" | "red_sandstone" => VoxelKind::Sand,
        "snow" | "snow_block" | "powder_snow" | "ice" | "packed_ice" | "blue_ice" => {
            VoxelKind::Snow
        }
        name if name.ends_with("_log") || name.ends_with("_wood") => VoxelKind::Wood,
        name if name.ends_with("_leaves") => VoxelKind::Leaves,
        // Lava is intentionally solid until the game has a lava material/behavior.
        _ => VoxelKind::Stone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_materials_preserve_air_water_and_solid_terrain() {
        assert_eq!(map_block("minecraft:cave_air"), VoxelKind::Air);
        assert_eq!(map_block("minecraft:water"), VoxelKind::Water);
        assert_eq!(map_block("minecraft:grass_block"), VoxelKind::Grass);
        assert_eq!(map_block("minecraft:deepslate"), VoxelKind::Deepslate);
        assert_eq!(map_block("minecraft:bedrock"), VoxelKind::Stone);
        assert_eq!(map_block("minecraft:lava"), VoxelKind::Stone);
    }

    #[test]
    fn decorated_forest_has_trees_ores_and_is_independent_of_worker_history() {
        let generator = WorldGenerator::new(42);
        let position = IVec2::new(-4, 0);
        let chunk = generator.with_worker(|worker| worker.chunk(position));
        let logs = chunk
            .iter()
            .filter(|&&kind| kind == VoxelKind::Wood)
            .count();
        let leaves = chunk
            .iter()
            .filter(|&&kind| kind == VoxelKind::Leaves)
            .count();
        let ores = chunk
            .iter()
            .filter(|&&kind| {
                matches!(
                    kind,
                    VoxelKind::CoalOre
                        | VoxelKind::IronOre
                        | VoxelKind::CopperOre
                        | VoxelKind::DiamondOre
                )
            })
            .count();
        eprintln!("forest chunk: {logs} logs, {leaves} leaves, {ores} ore blocks");
        assert!(logs > 0 && leaves > 0 && ores > 0);
        let fresh = std::thread::spawn(move || {
            WorldGenerator::new(42).with_worker(|worker| worker.chunk(position))
        })
        .join()
        .unwrap();
        assert_eq!(chunk, fresh);
        let spawn = generator.spawn_position();
        let position = IVec2::new(
            (spawn.x as i32).div_euclid(16),
            (spawn.z as i32).div_euclid(16),
        );
        let chunk = generator.with_worker(|worker| worker.chunk(position));
        let x = (spawn.x as i32).rem_euclid(16) as usize;
        let z = (spawn.z as i32).rem_euclid(16) as usize;
        assert!(chunk[index(x, spawn.y as usize - 3, z)].is_solid());
        assert_eq!(chunk[index(x, spawn.y as usize - 2, z)], VoxelKind::Air);
        let background = generator.clone();
        let generated = std::thread::spawn(move || {
            let result = generate_chunk(position, &VoxelSettings::default(), &background);
            // The spawn chunk and optional halo lookups must not initialize a new
            // generator or decorate unseen neighbors on the background worker.
            assert!(WORKER.with(|slot| slot.borrow().is_none()));
            result
        })
        .join()
        .unwrap();
        assert_eq!(
            generated.get(IVec3::new(x as i32, spawn.y as i32 - 3, z as i32)),
            Some(chunk[index(x, spawn.y as usize - 3, z)])
        );
    }

    #[test]
    fn generated_halo_matches_neighbor_and_negative_coordinates() {
        let settings = VoxelSettings::default();
        let generator = WorldGenerator::new(settings.seed);
        generator.chunk(IVec2::new(-1, 0));
        generator.chunk(IVec2::ZERO);
        let left = generate_chunk(IVec2::new(-1, 0), &settings, &generator);
        let right = generate_chunk(IVec2::ZERO, &settings, &generator);
        for y in 0..settings.max_height {
            for z in 0..16 {
                assert_eq!(
                    left.sample(IVec3::new(16, y, z)),
                    right.sample(IVec3::new(0, y, z))
                );
                assert_eq!(
                    right.sample(IVec3::new(-1, y, z)),
                    left.sample(IVec3::new(15, y, z))
                );
            }
        }
        assert!(generator.spawn_position().y > 64.0);
    }
}
