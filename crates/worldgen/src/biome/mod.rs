use sha2::{Digest, Sha256};
use std::cell::RefCell;

use pumpkin_data::chunk::{Biome, BiomeTree, NETHER_BIOME_SOURCE, OVERWORLD_BIOME_SOURCE};

use crate::generation::noise::router::multi_noise_sampler::MultiNoiseSampler;
pub mod end;
pub mod multi_noise;
pub mod position_finder;

pub use position_finder::{
    Climate, ClimateSampler, DistanceMetric, FittestPositionFinder, FittestPositionFinderResult,
    ParameterList, RTree, RTreeLeaf, RTreeNode, RTreeSubTree, SpawnFinder, SpawnFinderResult,
};
pub use pumpkin_data::chunk::{
    Parameter, ParameterPoint, ParameterRange, TargetPoint, quantize_coord, unquantize_coord,
};

thread_local! {
    /// A shortcut; check if last used biome is what we should use.
    ///
    /// Keyed by the tree the leaf came from. `BiomeTree::get` seeds its search distance from
    /// this leaf and only accepts strictly closer nodes, so a leaf cached from one dimension's
    /// tree would otherwise beat every node of another's and be returned unchanged. Vanilla
    /// stores `lastResult` on the `RTree` itself rather than sharing one across all of them.
    static LAST_RESULT_NODE: RefCell<Option<(&'static BiomeTree, &'static BiomeTree)>> =
        const { RefCell::new(None) };
}

pub trait BiomeSupplier {
    fn biome(&self, x: i32, y: i32, z: i32, noise: &mut MultiNoiseSampler<'_>) -> &'static Biome;
}

#[derive(Clone, Copy)]
pub struct MultiNoiseBiomeSupplier {
    source: &'static BiomeTree,
}

impl MultiNoiseBiomeSupplier {
    pub const OVERWORLD: Self = Self::new(&OVERWORLD_BIOME_SOURCE);
    pub const NETHER: Self = Self::new(&NETHER_BIOME_SOURCE);

    #[must_use]
    pub fn from_preset(preset: &str) -> Option<Self> {
        let preset = preset.strip_prefix("minecraft:").unwrap_or(preset);
        match preset {
            "overworld" | "large_biomes" | "amplified" => Some(Self::OVERWORLD),
            "nether" => Some(Self::NETHER),
            _ => None,
        }
    }

    const fn new(source: &'static BiomeTree) -> Self {
        Self { source }
    }
}

impl BiomeSupplier for MultiNoiseBiomeSupplier {
    fn biome(&self, x: i32, y: i32, z: i32, noise: &mut MultiNoiseSampler<'_>) -> &'static Biome {
        let point = noise.sample(x, y, z);
        let point_list = point.convert_to_list();
        LAST_RESULT_NODE.with_borrow_mut(|last_result| {
            let mut node = last_result
                .and_then(|(tree, node)| std::ptr::eq(tree, self.source).then_some(node));
            let biome = self.source.get(&point_list, &mut node);
            *last_result = node.map(|node| (self.source, node));
            biome
        })
    }
}

#[derive(Clone, Copy)]
pub struct FixedBiomeSupplier {
    pub biome: &'static Biome,
}

impl FixedBiomeSupplier {
    #[must_use]
    pub const fn new(biome: &'static Biome) -> Self {
        Self { biome }
    }
}

impl BiomeSupplier for FixedBiomeSupplier {
    fn biome(
        &self,
        _x: i32,
        _y: i32,
        _z: i32,
        _noise: &mut MultiNoiseSampler<'_>,
    ) -> &'static Biome {
        self.biome
    }
}

#[derive(Clone, Copy)]
pub enum ActiveBiomeSupplier {
    MultiNoise(MultiNoiseBiomeSupplier),
    End(end::TheEndBiomeSupplier),
    Fixed(FixedBiomeSupplier),
}

impl BiomeSupplier for ActiveBiomeSupplier {
    fn biome(&self, x: i32, y: i32, z: i32, noise: &mut MultiNoiseSampler<'_>) -> &'static Biome {
        match self {
            Self::MultiNoise(supplier) => supplier.biome(x, y, z, noise),
            Self::End(supplier) => supplier.biome(x, y, z, noise),
            Self::Fixed(supplier) => supplier.biome(x, y, z, noise),
        }
    }
}

impl ActiveBiomeSupplier {
    /// Exact set of reachable biomes, used to prune impossible structure sets.
    pub fn possible_biome_ids(&self) -> rustc_hash::FxHashSet<u16> {
        fn collect(tree: &BiomeTree, ids: &mut rustc_hash::FxHashSet<u16>) {
            match tree {
                BiomeTree::Leaf { biome, .. } => {
                    ids.insert(biome.id as u16);
                }
                BiomeTree::Branch { nodes, .. } => {
                    for node in *nodes {
                        collect(node, ids);
                    }
                }
            }
        }
        let mut ids = rustc_hash::FxHashSet::default();
        match self {
            Self::MultiNoise(supplier) => collect(supplier.source, &mut ids),
            Self::Fixed(supplier) => {
                ids.insert(supplier.biome.id as u16);
            }
            Self::End(_) => {
                for biome in [
                    Biome::THE_END,
                    Biome::END_HIGHLANDS,
                    Biome::END_MIDLANDS,
                    Biome::SMALL_END_ISLANDS,
                    Biome::END_BARRENS,
                ] {
                    ids.insert(biome.id as u16);
                }
            }
        }
        ids
    }

    #[must_use]
    pub fn from_biome_source(
        source: Option<&crate::biome::BiomeSource>,
        dimension: &pumpkin_data::dimension::Dimension,
    ) -> Self {
        match source {
            Some(crate::biome::BiomeSource::Fixed { biome, .. }) => {
                let clean = biome.strip_prefix("minecraft:").unwrap_or(biome);
                let resolved = pumpkin_data::biome::Biome::from_name(clean)
                    .unwrap_or(&pumpkin_data::biome::Biome::PLAINS);
                Self::Fixed(FixedBiomeSupplier::new(resolved))
            }
            Some(crate::biome::BiomeSource::WithPreset { preset, .. }) => {
                let clean = preset.strip_prefix("minecraft:").unwrap_or(preset);
                if clean == "nether" {
                    Self::MultiNoise(MultiNoiseBiomeSupplier::NETHER)
                } else {
                    Self::MultiNoise(MultiNoiseBiomeSupplier::OVERWORLD)
                }
            }
            Some(crate::biome::BiomeSource::Simple { biome_type }) => {
                let clean = biome_type.strip_prefix("minecraft:").unwrap_or(biome_type);
                if clean == "the_end" {
                    Self::End(end::TheEndBiomeSupplier)
                } else if clean == "the_nether" {
                    Self::MultiNoise(MultiNoiseBiomeSupplier::NETHER)
                } else {
                    Self::from_dimension(dimension)
                }
            }
            None => Self::from_dimension(dimension),
        }
    }

    #[must_use]
    pub fn from_dimension(dimension: &pumpkin_data::dimension::Dimension) -> Self {
        if dimension == &pumpkin_data::dimension::Dimension::THE_END {
            Self::End(end::TheEndBiomeSupplier)
        } else if dimension == &pumpkin_data::dimension::Dimension::THE_NETHER {
            Self::MultiNoise(MultiNoiseBiomeSupplier::NETHER)
        } else {
            Self::MultiNoise(MultiNoiseBiomeSupplier::OVERWORLD)
        }
    }
}

#[must_use]
pub fn hash_seed(seed: u64) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(seed.to_le_bytes());
    let result = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&result[..8]);
    i64::from_le_bytes(bytes)
}

mod source;
pub use source::BiomeSource;
