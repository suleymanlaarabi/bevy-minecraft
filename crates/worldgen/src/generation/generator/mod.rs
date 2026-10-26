use pumpkin_data::BlockState;
use pumpkin_data::dimension::Dimension;
use pumpkin_data::material_rule::MaterialRule;
use pumpkin_data::noise_router::BaseNoiseRouters;
use pumpkin_data::noise_settings::NoiseSettings;

use super::noise::router::proto_noise_router::ProtoNoiseRouters;
use crate::generation::proto_chunk::TerrainCache;
use crate::generation::{GlobalRandomConfig, Seed};

pub mod biome_finder;
pub mod structure_finder;

use pumpkin_data::structures::{StructurePlacementCalculator, StructureSet};
use rustc_hash::{FxHashMap, FxHashSet};

pub struct VanillaGenerator {
    pub fast_terrain: bool,
    pub(crate) structure_heights:
        super::noise::router::position_cache::PositionCache<(i32, i32, bool), [i32; 2]>,
    pub random_config: GlobalRandomConfig,
    pub base_router: ProtoNoiseRouters,
    pub dimension: Dimension,
    pub settings: &'static NoiseSettings,
    pub surface_rule: &'static MaterialRule,
    pub prepared_surface_rule: super::surface::rule::PreparedMaterialRule,
    pub identity_surface: bool,
    pub biome_supplier: crate::biome::ActiveBiomeSupplier,
    pub biome_mixer_seed: i64,

    pub terrain_cache: TerrainCache,

    pub default_block: &'static BlockState,

    pub global_structure_cache: crate::generation::structure::placement::GlobalStructureCache,
    pub structure_calculator: StructurePlacementCalculator,
    pub structure_allowed_biomes: FxHashMap<usize, Vec<u16>>,
    pub enabled_structure_sets: FxHashSet<usize>,
}

impl VanillaGenerator {
    #[must_use]
    pub fn find_spawn_position(&self) -> pumpkin_util::math::position::BlockPos {
        if self.settings.spawn_target.is_empty() {
            return pumpkin_util::math::position::BlockPos::ZERO;
        }
        let mut sampler =
            crate::generation::noise::router::multi_noise_sampler::MultiNoiseSampler::generate(
                &self.base_router.multi_noise,
            );
        crate::biome::position_finder::SpawnFinder::find_spawn_position(
            self.settings.spawn_target,
            &mut sampler,
        )
    }

    #[must_use]
    pub fn new(seed: Seed, dimension: Dimension) -> Self {
        let settings = NoiseSettings::from_dimension(&dimension);
        let surface_rule = MaterialRule::from_dimension(&dimension);
        let biome_supplier = crate::biome::ActiveBiomeSupplier::from_dimension(&dimension);
        let random_config = GlobalRandomConfig::new(seed.0, settings.legacy_random_source);

        let base = BaseNoiseRouters::from_dimension(&dimension);
        let terrain_cache = TerrainCache::from_random(&random_config);

        let default_block = settings.default_block;
        let base_router = ProtoNoiseRouters::generate(base, &random_config);
        let biome_mixer_seed = crate::biome::hash_seed(seed.0);

        let mut structure_allowed_biomes = FxHashMap::default();
        for (i, set) in StructureSet::ALL.iter().enumerate() {
            structure_allowed_biomes.insert(
                i,
                crate::generation::proto_chunk::ProtoChunk::get_allowed_biomes(set),
            );
        }

        let possible_biomes = biome_supplier.possible_biome_ids();
        let identity_surface = matches!(surface_rule, MaterialRule::Block(rule) if rule.result_state.id == default_block.id)
            && ![
                pumpkin_data::chunk::Biome::ERODED_BADLANDS,
                pumpkin_data::chunk::Biome::FROZEN_OCEAN,
                pumpkin_data::chunk::Biome::DEEP_FROZEN_OCEAN,
            ]
            .iter()
            .any(|biome| possible_biomes.contains(&(biome.id as u16)))
            && std::env::var("WORLDGEN_IDENTITY_SURFACE").as_deref() != Ok("off");
        let enabled_structure_sets = structure_allowed_biomes
            .iter()
            .filter(|(_, biomes)| biomes.iter().any(|biome| possible_biomes.contains(biome)))
            .map(|(&index, _)| index)
            .collect();

        let global_structure_cache = if dimension == Dimension::OVERWORLD {
            let pumpkin_data::structures::StructurePlacementType::ConcentricRings(rings) =
                &StructureSet::STRONGHOLDS.placement.placement_type
            else {
                unreachable!()
            };
            crate::generation::structure::placement::GlobalStructureCache::with_background_strongholds(
                seed.0 as i64,
                rings,
                &base_router.multi_noise,
            )
        } else {
            crate::generation::structure::placement::GlobalStructureCache::new()
        };

        Self {
            random_config,
            base_router,
            dimension,
            settings,
            surface_rule,
            prepared_surface_rule: super::surface::rule::PreparedMaterialRule::new(surface_rule),
            identity_surface,
            biome_supplier,
            biome_mixer_seed,
            terrain_cache,
            default_block,
            global_structure_cache,
            structure_calculator: StructurePlacementCalculator::new(seed.0 as i64),
            structure_allowed_biomes,
            fast_terrain: false,
            structure_heights: super::noise::router::position_cache::PositionCache::default(),
            enabled_structure_sets,
        }
    }
}
