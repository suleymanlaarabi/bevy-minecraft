use super::*;
use crate::{
    block_behavior::containers::{BlockEntity, GameplayColumn, GameplayColumns},
    character::{CharacterBody, CharacterState, Pose, Terrain},
    simulation::{GameplayClock, SimulatedEntities, SimulatedIn, Simulating},
    survival::Health,
    voxel::{VoxelKind, VoxelWorld},
};
use bevy::{ecs::system::SystemParam, math::DVec3, prelude::*};
use hollow_features::motion::FixedPosition;
use std::collections::VecDeque;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[component(immutable)]
pub enum SpawnSource {
    Natural,
    Spawner,
    Developer,
}

/// A zero-sized archetype marker lets population caps count natural animals
/// without visiting every developer or spawner creature.
#[derive(Component)]
struct NaturalCreature;

#[derive(Event, Clone, Copy)]
pub struct SpawnCreature {
    pub species: SpeciesId,
    /// Feet position; natural spawning projects this x/z onto the terrain surface.
    pub feet: Vec3,
    pub source: SpawnSource,
    pub population_center: Vec3,
}

/// Begin a bounded, progressive developer population request around the player.
#[derive(Event)]
pub struct SpawnCreaturePopulation(pub u32);

#[derive(Component, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreatureSpawner {
    pub species: SpeciesId,
    pub delay: u16,
}

/// Safety limit for active creatures; natural spawning retains its own cap.
#[derive(Resource)]
pub struct CreaturePopulationLimit(pub usize);

impl Default for CreaturePopulationLimit {
    fn default() -> Self {
        Self(16_384)
    }
}
impl Default for CreatureSpawner {
    fn default() -> Self {
        Self {
            species: SpeciesId(EntityType::PIG.id),
            delay: 200,
        }
    }
}

#[derive(Component, Clone)]
pub(crate) struct ColumnBiomes(pub std::sync::Arc<[u8]>);
impl ColumnBiomes {
    fn at(&self, position: IVec3, height: i32) -> Option<u8> {
        if position.y < 0 || position.y >= height as i32 {
            return None;
        }
        let quart_height = height as usize / 4;
        let index = quart_height * 4 * (position.x.rem_euclid(16) as usize / 4)
            + 4 * (position.y as usize / 4)
            + position.z.rem_euclid(16) as usize / 4;
        self.0.get(index).copied()
    }
}
enum Candidate {
    Natural(Vec3),
    Defined(SpawnCreature),
    Population(Vec3, SpeciesId),
}
#[derive(Resource, Default)]
struct Requests(VecDeque<Candidate>);
#[derive(Resource, Default)]
struct PopulationRequest {
    center: Vec3,
    remaining: u32,
    attempts: u32,
}
#[derive(SystemParam)]
struct SpawnEnvironment<'w> {
    settings: Res<'w, hollow_worldgen::GenerationSettings>,
    lighting: Res<'w, hollow_engine::lighting::runtime::LightingState<crate::voxel::VoxelState>>,
    lighting_version: Res<'w, hollow_engine::lighting::runtime::LightingVersion>,
}
#[derive(SystemParam)]
struct PopulationControl<'w, 's> {
    population: ResMut<'w, PopulationRequest>,
    limit: Res<'w, CreaturePopulationLimit>,
    active: Query<'w, 's, (), (With<Creature>, With<Simulating>)>,
    natural: Query<'w, 's, (), (With<NaturalCreature>, With<Simulating>)>,
}
#[derive(Resource, Default)]
struct SpawnBudget {
    candidates: usize,
    creations: usize,
}
fn reset(mut budget: ResMut<SpawnBudget>) {
    budget.candidates = 8;
    budget.creations = 4;
}
fn request(event: On<SpawnCreature>, mut requests: ResMut<Requests>) {
    // Bound external producers as well as world work; no unbounded entity backlog.
    if requests.0.len() < 128 {
        requests.0.push_back(Candidate::Defined(*event));
    }
}
fn compile_biomes(mut catalog: ResMut<CreatureCatalog>) {
    for biome in pumpkin_data::biome::Biome::ALL {
        for entry in biome.spawners.creature {
            let Some(native) = EntityType::from_name(
                entry
                    .r#type
                    .strip_prefix("minecraft:")
                    .unwrap_or(entry.r#type),
            ) else {
                continue;
            };
            if catalog.get(SpeciesId(native.id)).is_some() {
                catalog.biomes[biome.id as usize].push(SpawnEntry {
                    species: SpeciesId(native.id),
                    minimum: entry.min_count.clamp(1, 4) as u8,
                    maximum: entry.max_count.clamp(1, 4) as u8,
                });
            }
        }
    }
}
impl Plugin for CreatureSpawningPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Requests>()
            .init_resource::<SpawnBudget>()
            .init_resource::<CreaturePopulationLimit>()
            .init_resource::<PopulationRequest>()
            .add_observer(request)
            .add_observer(request_population)
            .add_observer(spawner_removed)
            .add_systems(
                OnEnter(crate::game::GameState::Game),
                seed_random.after(crate::world::PrepareSession),
            )
            .add_systems(Startup, compile_biomes)
            .add_systems(
                RunFixedMainLoop,
                reset.in_set(bevy::app::RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(
                FixedUpdate,
                (natural, spawners, populate, process)
                    .chain()
                    .in_set(CreatureSystems::Spawn),
            )
            .add_systems(OnExit(crate::game::GameState::Game), clear);
        app.world_mut()
            .resource_mut::<hollow_engine::blocks::BlockRegistrations>()
            .register(
                hollow_core::BlockId(
                    VoxelKind::from_native(pumpkin_data::Block::SPAWNER.id).index() as u32,
                ),
                hollow_engine::blocks::BlockFactory {
                    attach: |entity, _| {
                        entity.insert(CreatureSpawner::default());
                    },
                    detach: |entity| {
                        entity.remove::<CreatureSpawner>();
                    },
                    interactive: false,
                    refresh_on_state_change: false,
                },
            );
    }
}
fn clear(mut requests: ResMut<Requests>, mut population: ResMut<PopulationRequest>) {
    requests.0.clear();
    *population = PopulationRequest::default();
}

fn request_population(
    event: On<SpawnCreaturePopulation>,
    player: Query<&FixedPosition, With<crate::player::Player>>,
    mut population: ResMut<PopulationRequest>,
    mut requests: ResMut<Requests>,
    mut status: Option<ResMut<crate::devtools::CreatureSpawnStatus>>,
) {
    let Ok(player) = player.single() else { return };
    requests
        .0
        .retain(|candidate| !matches!(candidate, Candidate::Population(..)));
    *population = PopulationRequest {
        center: player.current.as_vec3(),
        remaining: event.0.min(16_384),
        attempts: 0,
    };
    if let Some(status) = status.as_mut() {
        status.0 = "Population spawning started.";
    }
}

fn populate(mut population: ResMut<PopulationRequest>, mut requests: ResMut<Requests>) {
    // A permutation of a 128 x 128 lattice spreads attempts without storing positions.
    // Stop after four passes if terrain or occupancy prevents the requested population.
    while population.remaining > 0 && requests.0.len() < 8 && population.attempts < 65_536 {
        let index = population.attempts.wrapping_mul(8191) % 16_384;
        let offset = Vec3::new(
            (index % 128) as f32 * 1.5 - 95.25,
            0.0,
            (index / 128) as f32 * 1.5 - 95.25,
        );
        let species = if population.attempts % 2 == 0 {
            COW.native.id
        } else {
            PIG.native.id
        };
        requests.0.push_back(Candidate::Population(
            population.center + offset,
            SpeciesId(species),
        ));
        population.attempts += 1;
    }
}

fn natural(
    clock: Res<GameplayClock>,
    player: Query<&FixedPosition, With<crate::player::Player>>,
    active: Query<(), (With<NaturalCreature>, With<Simulating>)>,
    mut random: ResMut<CreatureRandom>,
    mut requests: ResMut<Requests>,
) {
    if clock.0 % 20 != 0
        || requests.0.len() >= 8
        || active.iter().len() >= EntityType::COW.category.max as usize
    {
        return;
    }
    let Ok(player) = player.single() else {
        return;
    };
    for _ in 0..8 {
        if requests.0.len() >= 8 {
            break;
        }
        let offset = IVec2::new(random.range(193) as i32 - 96, random.range(193) as i32 - 96);
        if !(24 * 24..=96 * 96).contains(&offset.length_squared()) {
            continue;
        }
        let at = player.current.floor().as_vec3()
            + Vec3::new(offset.x as f32 + 0.5, 0.0, offset.y as f32 + 0.5);
        requests.0.push_back(Candidate::Natural(at));
    }
}

fn surface(world: &VoxelWorld, mut feet: Vec3) -> Option<Vec3> {
    let at = feet.floor().as_ivec3();
    let mut surface = None;
    // One resolved column, at most the finite world height, inside the shared
    // candidate budget. No height cache or repeated per-cell hash lookup.
    world.visit_cells(
        IVec3::new(at.x, 0, at.z),
        IVec3::new(at.x, world.layout().height() - 1, at.z),
        |p, state| {
            if surface.is_none() && state.is_some_and(|s| !s.collision_empty() || s.is_liquid()) {
                surface = Some(p.y);
            }
        },
    );
    feet.y = surface? as f32 + 1.0;
    Some(feet)
}

fn spawners(
    player: Query<&FixedPosition, With<crate::player::Player>>,
    mut spawners: Query<(&BlockEntity, &mut CreatureSpawner), With<Simulating>>,
    mut random: ResMut<CreatureRandom>,
    mut requests: ResMut<Requests>,
) {
    let Ok(player) = player.single() else {
        return;
    };
    for (block, mut spawner) in &mut spawners {
        let position = block.position.as_vec3() + Vec3::splat(0.5);
        if player.current.as_vec3().distance_squared(position) > 256.0 {
            continue;
        }
        if spawner.delay > 0 {
            // The countdown is saved with the next column capture but never dirties it.
            spawner.bypass_change_detection().delay -= 1;
            continue;
        }
        spawner.delay = (200 + random.range(601)) as u16;
        for _ in 0..4 {
            if requests.0.len() >= 128 {
                break;
            }
            let offset = Vec3::new(
                random.range(9) as f32 - 4.0,
                random.range(3) as f32 - 1.0,
                random.range(9) as f32 - 4.0,
            );
            requests.0.push_back(Candidate::Defined(SpawnCreature {
                species: spawner.species,
                feet: position + offset - Vec3::Y * 0.5,
                source: SpawnSource::Spawner,
                population_center: position,
            }));
        }
    }
}

#[derive(Clone, Copy)]
enum Rejection {
    Cap,
    Terrain,
    Space,
    Occupied,
    Biome,
    Light,
    Distance,
}
impl Rejection {
    fn record(self, diagnostics: &mut crate::simulation::SimulationDiagnostics) {
        diagnostics.spawn_rejections[self as usize] += 1;
    }
}

fn process(
    mut commands: Commands,
    mut requests: ResMut<Requests>,
    control: PopulationControl,
    mut budget: ResMut<SpawnBudget>,
    catalog: Res<CreatureCatalog>,
    world: VoxelWorld,
    columns: Res<GameplayColumns>,
    owners: Query<(&GameplayColumn, Option<&SimulatedEntities>, &ColumnBiomes), With<Simulating>>,
    actors: Query<(&FixedPosition, &CharacterBody, &SpawnSource), With<Creature>>,
    player: Query<(&FixedPosition, &CharacterBody, &CharacterState), With<crate::player::Player>>,
    environment: SpawnEnvironment,
    mut random: ResMut<CreatureRandom>,
    mut diagnostics: ResMut<crate::simulation::SimulationDiagnostics>,
    mut developer_status: Option<ResMut<crate::devtools::CreatureSpawnStatus>>,
    mut terrain: Local<Terrain>,
) {
    let PopulationControl {
        mut population,
        limit,
        active,
        natural,
    } = control;
    if requests.0.is_empty() || budget.candidates == 0 || budget.creations == 0 {
        return;
    }
    let mut total = active.iter().len();
    let mut natural = natural.iter().len();
    // Four deferred creations are counted immediately; no global population cache.
    let mut reserved = [(
        Entity::PLACEHOLDER,
        crate::character::Aabb {
            min: DVec3::ZERO,
            max: DVec3::ZERO,
        },
        SpawnSource::Natural,
    ); 4];
    let mut count = 0;
    while budget.candidates > 0 && budget.creations > 0 {
        let Some(candidate) = requests.0.pop_front() else {
            break;
        };
        budget.candidates -= 1;
        let developer = matches!(&candidate, Candidate::Defined(request) if request.source == SpawnSource::Developer);
        let population_candidate = matches!(&candidate, Candidate::Population(..));
        if population_candidate && population.remaining == 0 {
            continue;
        }
        let outcome = (|| {
            let request = match candidate {
                Candidate::Population(at, species) => SpawnCreature {
                    species,
                    feet: surface(&world, at).ok_or(Rejection::Terrain)?,
                    source: SpawnSource::Developer,
                    population_center: population.center,
                },
                Candidate::Defined(mut request) => {
                    if request.source == SpawnSource::Natural {
                        if total >= limit.0 || natural >= EntityType::COW.category.max as usize {
                            return Err(Rejection::Cap);
                        }
                        request.feet = surface(&world, request.feet).ok_or(Rejection::Terrain)?;
                    }
                    request
                }
                Candidate::Natural(at) => {
                    if total >= limit.0 || natural >= EntityType::COW.category.max as usize {
                        return Err(Rejection::Cap);
                    }
                    let Some(owner) = columns.get(&crate::voxel::chunk_at(at)) else {
                        return Err(Rejection::Terrain);
                    };
                    let Ok((_, _, biomes)) = owners.get(*owner) else {
                        return Err(Rejection::Terrain);
                    };
                    let feet = surface(&world, at).ok_or(Rejection::Terrain)?;
                    let biome = biomes
                        .at((feet - Vec3::Y).floor().as_ivec3(), world.layout().height())
                        .ok_or(Rejection::Biome)?;
                    let entries = &catalog.biomes[biome as usize];
                    if entries.is_empty() {
                        return Err(Rejection::Biome);
                    }
                    let entry = entries[random.range(entries.len() as u32) as usize];
                    let group = entry.minimum
                        + random.range(u32::from(entry.maximum - entry.minimum + 1)) as u8;
                    for _ in 1..group {
                        if requests.0.len() >= 128 {
                            break;
                        }
                        let jitter = Vec3::new(
                            random.range(7) as f32 - 3.0,
                            0.0,
                            random.range(7) as f32 - 3.0,
                        );
                        requests.0.push_front(Candidate::Defined(SpawnCreature {
                            species: entry.species,
                            feet: feet + jitter,
                            source: SpawnSource::Natural,
                            population_center: feet,
                        }));
                    }
                    SpawnCreature {
                        species: entry.species,
                        feet,
                        source: SpawnSource::Natural,
                        population_center: feet,
                    }
                }
            };
            let Some(profile) = catalog.get(request.species) else {
                return Err(Rejection::Biome);
            };
            if total >= limit.0
                || (request.source == SpawnSource::Natural
                    && natural >= EntityType::COW.category.max as usize)
            {
                return Err(Rejection::Cap);
            }
            if !request.feet.is_finite()
                || !request.population_center.is_finite()
                || profile.dimension != environment.settings.dimension
            {
                return Err(Rejection::Terrain);
            }
            let coordinate = crate::voxel::chunk_at(request.feet);
            let Some(&owner) = columns.get(&coordinate) else {
                return Err(Rejection::Terrain);
            };
            let Ok((_, members, biomes)) = owners.get(owner) else {
                return Err(Rejection::Terrain);
            };
            let body = profile.body;
            let origin = request.feet + Vec3::Y * body.origin_height;
            // Preserve the support face when adding a fractional body origin:
            // f32 rounding at world height can otherwise embed the feet in the ground.
            let precise_origin = request.feet.as_dvec3() + DVec3::Y * body.origin_height as f64;
            let bounds = body.aabb(precise_origin, Pose::Standing);
            terrain.gather(&world, bounds.expanded_towards(DVec3::NEG_Y * 0.05));
            if terrain.collides(&bounds)
                || terrain.has_fluid(&bounds)
                || terrain.collide_box(bounds, DVec3::NEG_Y * 0.05).y < -0.049
            {
                return Err(Rejection::Space);
            }
            if request.source == SpawnSource::Natural {
                let column_natural = members
                    .into_iter()
                    .flat_map(|m| m.iter())
                    .filter_map(|entity| actors.get(entity).ok())
                    .filter(|(_, _, source)| **source == SpawnSource::Natural)
                    .count()
                    + reserved[..count]
                        .iter()
                        .filter(|(column, _, source)| {
                            *column == owner && *source == SpawnSource::Natural
                        })
                        .count();
                if column_natural >= profile.native.limit_per_chunk as usize {
                    return Err(Rejection::Cap);
                }
            }
            let mut nearby = 0;
            let center = if request.source == SpawnSource::Spawner {
                request.population_center
            } else {
                request.feet
            };
            let center_column = crate::voxel::chunk_at(center);
            // Other creatures can overlap across a column boundary by their own half-width.
            // Spawners also need the full 16-block population radius.
            let reach = DVec3::splat(catalog.maximum_half_width as f64);
            let mut overlap_min = crate::voxel::chunk_at((bounds.min - reach).as_vec3());
            let mut overlap_max = crate::voxel::chunk_at((bounds.max + reach).as_vec3());
            if request.source == SpawnSource::Spawner {
                overlap_min = overlap_min.min(center_column - IVec2::ONE);
                overlap_max = overlap_max.max(center_column + IVec2::ONE);
            }
            for x in overlap_min.x..=overlap_max.x {
                for z in overlap_min.y..=overlap_max.y {
                    let coordinate = IVec2::new(x, z);
                    let Some(other) = columns.get(&coordinate) else {
                        continue;
                    };
                    let Ok((_, Some(members), _)) = owners.get(*other) else {
                        continue;
                    };
                    for entity in members.iter() {
                        if let Ok((position, other_body, _)) = actors.get(entity) {
                            if position.current.as_vec3().distance_squared(center) <= 256.0 {
                                nearby += 1;
                            }
                            if bounds.intersects(&other_body.aabb(position.current, Pose::Standing))
                            {
                                return Err(Rejection::Occupied);
                            }
                        }
                    }
                }
            }
            for (_, other, _) in &reserved[..count] {
                let position = ((other.min + other.max) * 0.5).as_vec3();
                if position.distance_squared(center) <= 256.0 {
                    nearby += 1;
                }
                if bounds.intersects(other) {
                    return Err(Rejection::Occupied);
                }
            }
            if request.source == SpawnSource::Spawner && nearby >= 6 {
                return Err(Rejection::Cap);
            }
            if let Ok((player, player_body, player_state)) = player.single() {
                if player_body
                    .aabb(player.current, player_state.pose)
                    .intersects(&bounds)
                {
                    return Err(Rejection::Occupied);
                }
                if request.source == SpawnSource::Spawner
                    && player
                        .current
                        .as_vec3()
                        .distance_squared(request.population_center)
                        > 256.0
                {
                    return Err(Rejection::Distance);
                }
                if request.source == SpawnSource::Natural
                    && !(24.0f32.powi(2)..=96.0f32.powi(2))
                        .contains(&player.current.as_vec3().distance_squared(origin))
                {
                    return Err(Rejection::Distance);
                }
            } else {
                return Err(Rejection::Distance);
            }
            if request.source == SpawnSource::Natural {
                let support = (request.feet - Vec3::Y * 0.01).floor().as_ivec3();
                if world.get(support)
                    != Some(VoxelKind::from_native(pumpkin_data::Block::GRASS_BLOCK.id))
                {
                    return Err(Rejection::Biome);
                }
                let Some(biome) = biomes.at(support, world.layout().height()) else {
                    return Err(Rejection::Biome);
                };
                if !catalog.biomes[biome as usize]
                    .iter()
                    .any(|entry| entry.species == request.species)
                {
                    return Err(Rejection::Biome);
                }
                // The lighting worker commits edit neighborhoods together; only
                // columns inside one still being relit wait for its snapshot.
                if !environment
                    .lighting
                    .is_settled(coordinate, &environment.lighting_version)
                {
                    return Err(Rejection::Light);
                }
                let Some(light) = world.gameplay_light(request.feet.floor().as_ivec3()) else {
                    return Err(Rejection::Light);
                };
                // Animals use raw brightness: stored sky light, not darkened by time of day.
                if (light >> 4).max(light & 15) < 9 {
                    return Err(Rejection::Light);
                }
            }
            Ok((owner, precise_origin, profile, request.source))
        })();
        match outcome {
            Ok((owner, origin, profile, source)) => {
                spawn(&mut commands, profile, origin, source, owner);
                if population_candidate {
                    population.remaining = population.remaining.saturating_sub(1);
                    if population.remaining == 0
                        && let Some(status) = developer_status.as_mut()
                    {
                        status.0 = "Population spawning complete.";
                    }
                }
                if developer && let Some(status) = developer_status.as_mut() {
                    status.0 = "Creature spawned in front of you.";
                }
                reserved[count] = (owner, profile.body.aabb(origin, Pose::Standing), source);
                count += 1;
                budget.creations -= 1;
                total += 1;
                natural += usize::from(source == SpawnSource::Natural);
            }
            Err(reason) => {
                if population_candidate && matches!(reason, Rejection::Cap) {
                    population.remaining = 0;
                    if let Some(status) = developer_status.as_mut() {
                        status.0 = "Population spawning stopped at the creature limit.";
                    }
                }
                if developer && let Some(status) = developer_status.as_mut() {
                    status.0 = match reason {
                        Rejection::Cap => "Creature limit reached.",
                        Rejection::Terrain => "Terrain is not ready. Try again nearby.",
                        Rejection::Space | Rejection::Occupied => {
                            "Spawn position is blocked. Try again."
                        }
                        Rejection::Biome | Rejection::Light | Rejection::Distance => {
                            "Cannot spawn here. Try another location."
                        }
                    };
                }
                reason.record(&mut diagnostics);
            }
        }
    }
    if population.remaining > 0
        && population.attempts >= 65_536
        && !requests
            .0
            .iter()
            .any(|candidate| matches!(candidate, Candidate::Population(..)))
    {
        population.remaining = 0;
        if let Some(status) = developer_status.as_mut() {
            status.0 = "Population spawning stopped: insufficient suitable space.";
        }
    }
}

pub(crate) fn spawn(
    commands: &mut Commands,
    profile: &CreatureProfile,
    origin: DVec3,
    source: SpawnSource,
    owner: Entity,
) -> Entity {
    let health = attribute(profile.native, &Attributes::MAX_HEALTH, 10.0) as f32;
    let mut position = FixedPosition::new(origin.as_vec3());
    position.current = origin;
    position.previous = origin;
    let mut entity = commands.spawn((
        Creature(SpeciesId(profile.native.id)),
        profile.body,
        source,
        position,
        Transform::from_translation(origin.as_vec3()),
        Visibility::default(),
        Health {
            current: health,
            maximum: health,
            absorption: 0.0,
        },
        CharacterState::default(),
        SimulatedIn(owner),
        DespawnOnExit(crate::game::GameState::Game),
    ));
    if source == SpawnSource::Natural {
        entity.insert(NaturalCreature);
    }
    (profile.compose)(&mut entity);
    entity.id()
}

fn spawner_removed(
    event: On<Remove<CreatureSpawner>>,
    blocks: Query<&crate::block_behavior::containers::BlockOf>,
    mut columns: Query<&mut GameplayColumn>,
) {
    if let Ok(owner) = blocks.get(event.entity)
        && let Ok(mut column) = columns.get_mut(owner.0)
    {
        column.revision += 1;
    }
}
fn seed_random(
    session: Res<hollow_worldgen::GenerationSession>,
    mut random: ResMut<CreatureRandom>,
) {
    random.0 = ((session.settings().seed as u64) ^ 0x34d57f918e625cab).max(1);
}
