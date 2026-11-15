use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use pumpkin_data::chunk::Biome;
use pumpkin_data::placed_feature::PlacedFeature;

// The order of biomes in this array is used by FeatureSorter to break ties between otherwise independent features.
const OVERWORLD_BIOMES: &[&Biome] = &[
    &Biome::MUSHROOM_FIELDS,
    &Biome::DEEP_FROZEN_OCEAN,
    &Biome::FROZEN_OCEAN,
    &Biome::DEEP_COLD_OCEAN,
    &Biome::COLD_OCEAN,
    &Biome::DEEP_OCEAN,
    &Biome::OCEAN,
    &Biome::DEEP_LUKEWARM_OCEAN,
    &Biome::LUKEWARM_OCEAN,
    &Biome::WARM_OCEAN,
    &Biome::STONY_SHORE,
    &Biome::SWAMP,
    &Biome::MANGROVE_SWAMP,
    &Biome::SNOWY_SLOPES,
    &Biome::SNOWY_PLAINS,
    &Biome::SNOWY_BEACH,
    &Biome::WINDSWEPT_GRAVELLY_HILLS,
    &Biome::GROVE,
    &Biome::WINDSWEPT_HILLS,
    &Biome::SNOWY_TAIGA,
    &Biome::WINDSWEPT_FOREST,
    &Biome::TAIGA,
    &Biome::PLAINS,
    &Biome::MEADOW,
    &Biome::BEACH,
    &Biome::FOREST,
    &Biome::OLD_GROWTH_SPRUCE_TAIGA,
    &Biome::FLOWER_FOREST,
    &Biome::BIRCH_FOREST,
    &Biome::DARK_FOREST,
    &Biome::PALE_GARDEN,
    &Biome::SAVANNA_PLATEAU,
    &Biome::SAVANNA,
    &Biome::JUNGLE,
    &Biome::BADLANDS,
    &Biome::DESERT,
    &Biome::WOODED_BADLANDS,
    &Biome::JAGGED_PEAKS,
    &Biome::STONY_PEAKS,
    &Biome::FROZEN_RIVER,
    &Biome::RIVER,
    &Biome::ICE_SPIKES,
    &Biome::OLD_GROWTH_PINE_TAIGA,
    &Biome::SUNFLOWER_PLAINS,
    &Biome::OLD_GROWTH_BIRCH_FOREST,
    &Biome::SPARSE_JUNGLE,
    &Biome::BAMBOO_JUNGLE,
    &Biome::ERODED_BADLANDS,
    &Biome::WINDSWEPT_SAVANNA,
    &Biome::CHERRY_GROVE,
    &Biome::FROZEN_PEAKS,
    &Biome::DRIPSTONE_CAVES,
    &Biome::LUSH_CAVES,
    &Biome::SULFUR_CAVES,
    &Biome::DEEP_DARK,
    &Biome::DAPPLED_FOREST,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FeatureData {
    step: usize,
    encounter_index: usize,
    feature: PlacedFeature,
}

impl Ord for FeatureData {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.step, self.encounter_index, self.feature).cmp(&(
            other.step,
            other.encounter_index,
            other.feature,
        ))
    }
}

impl PartialOrd for FeatureData {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

const NETHER_BIOMES: &[&Biome] = &[
    &Biome::NETHER_WASTES,
    &Biome::SOUL_SAND_VALLEY,
    &Biome::CRIMSON_FOREST,
    &Biome::WARPED_FOREST,
    &Biome::BASALT_DELTAS,
];

const END_BIOMES: &[&Biome] = &[
    &Biome::THE_END,
    &Biome::END_HIGHLANDS,
    &Biome::END_MIDLANDS,
    &Biome::SMALL_END_ISLANDS,
    &Biome::END_BARRENS,
];

type BiomeMask = [u64; 4];

struct FeatureCatalog {
    biomes: BiomeMask,
    steps: Vec<Vec<(PlacedFeature, BiomeMask)>>,
}

impl FeatureCatalog {
    fn new(biomes: &[&Biome]) -> Self {
        let mut mask = [0; 4];
        for biome in biomes {
            mask[usize::from(biome.id) / 64] |= 1 << (biome.id % 64);
        }
        let steps = sort_features_per_step(biomes)
            .into_iter()
            .enumerate()
            .map(|(step, features)| {
                features
                    .into_iter()
                    .map(|feature| {
                        let mut mask = [0; 4];
                        for biome in biomes {
                            if biome
                                .features
                                .get(step)
                                .is_some_and(|features| features.contains(&feature))
                            {
                                mask[usize::from(biome.id) / 64] |= 1 << (biome.id % 64);
                            }
                        }
                        (feature, mask)
                    })
                    .collect()
            })
            .collect();
        Self {
            biomes: mask,
            steps,
        }
    }
}

// Resolve biome membership once. Selection only scans compact masks in the
// original global feature order, preserving decorator indices and random seeds.
static CATALOGS: LazyLock<[FeatureCatalog; 3]> =
    LazyLock::new(|| [NETHER_BIOMES, END_BIOMES, OVERWORLD_BIOMES].map(FeatureCatalog::new));

fn intersects(a: &BiomeMask, b: &BiomeMask) -> bool {
    a.iter().zip(b).any(|(a, b)| a & b != 0)
}

pub fn select_features(
    biomes: &BiomeMask,
    step: usize,
) -> impl Iterator<Item = (usize, PlacedFeature)> {
    let catalog = CATALOGS[..2]
        .iter()
        .find(|catalog| intersects(biomes, &catalog.biomes))
        .unwrap_or(&CATALOGS[2]);
    catalog
        .steps
        .get(step)
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, (_, mask))| intersects(biomes, mask))
        .map(|(index, (feature, _))| (index, *feature))
}

fn sort_features_per_step(biomes: &[&Biome]) -> Vec<Vec<PlacedFeature>> {
    let mut feature_indices = HashMap::new();
    let mut edges: BTreeMap<FeatureData, BTreeSet<FeatureData>> = BTreeMap::new();
    let mut max_steps = 0;

    for biome in biomes {
        max_steps = max_steps.max(biome.features.len());
        let mut biome_features = Vec::new();

        for (step, features) in biome.features.iter().enumerate() {
            for &feature in *features {
                let next_index = feature_indices.len();
                let encounter_index = *feature_indices.entry(feature).or_insert(next_index);
                let data = FeatureData {
                    step,
                    encounter_index,
                    feature,
                };
                edges.entry(data).or_default();
                biome_features.push(data);
            }
        }

        for pair in biome_features.windows(2) {
            edges.entry(pair[0]).or_default().insert(pair[1]);
        }
    }

    let mut discovered = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    let mut sorted = Vec::with_capacity(edges.len());
    for &feature in edges.keys() {
        visit_feature(feature, &edges, &mut discovered, &mut visiting, &mut sorted);
    }
    sorted.reverse();

    let mut per_step = vec![Vec::new(); max_steps];
    for feature in sorted {
        per_step[feature.step].push(feature.feature);
    }
    per_step
}

fn visit_feature(
    feature: FeatureData,
    edges: &BTreeMap<FeatureData, BTreeSet<FeatureData>>,
    discovered: &mut BTreeSet<FeatureData>,
    visiting: &mut BTreeSet<FeatureData>,
    sorted: &mut Vec<FeatureData>,
) {
    if discovered.contains(&feature) {
        return;
    }
    assert!(
        visiting.insert(feature),
        "feature order cycle contains {feature:?}"
    );
    if let Some(next_features) = edges.get(&feature) {
        for &next in next_features {
            visit_feature(next, edges, discovered, visiting, sorted);
        }
    }
    visiting.remove(&feature);
    discovered.insert(feature);
    sorted.push(feature);
}
