//! Fixed gameplay time, spatial activation and shared per-frame work limits.
mod fluids;
mod work;
pub(crate) use work::*;

use crate::{
    block_behavior::containers::{BlockEntities, GameplayColumn, GameplayColumns, LoadedColumn},
    character::CharacterMotion,
    player::Player,
    settings::SimulationSettings,
    voxel::{VoxelChunk, chunk_at, streaming::ChunkIndex},
};
use bevy::prelude::*;
use std::time::Duration;

#[derive(Resource, Default, Clone, Copy)]
pub struct GameplayClock(pub u64);

#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct Simulating;

#[derive(Component)]
#[relationship(relationship_target = SimulatedEntities)]
pub struct SimulatedIn(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = SimulatedIn)]
pub struct SimulatedEntities(Vec<Entity>);

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SimulationSystems {
    Activation,
    Neighbors,
    Scheduled,
    Random,
    Machines,
    Actors,
    Finalize,
}

#[derive(Resource, Default)]
pub(crate) struct SimulationRegion {
    pub center: Option<IVec2>,
    pub radius: u32,
}
impl SimulationRegion {
    pub fn contains(&self, position: IVec2) -> bool {
        self.center.is_some_and(|center| {
            hollow_core::coordinates::chunk_distance_squared(position, center)
                <= (self.radius * self.radius) as i32
        })
    }
}

#[derive(Resource, Default)]
pub struct SimulationDiagnostics {
    pub active_columns: usize,
    pub neighbor_cells: usize,
    pub scheduled_ticks: usize,
    pub modified_cells: usize,
    pub random_attempts: usize,
    pub ready_work: usize,
    pub suspended_work: usize,
    pub pending_write_bytes: usize,
    pub maximum_lateness: u64,
}
#[derive(Resource, Default)]
pub(crate) struct SimulationBudget {
    pub neighbors: usize,
    pub scheduled: usize,
    pub mutations: usize,
    pub random: usize,
}
pub(crate) const MAX_PENDING_WRITE_BYTES: usize = 16 * 1024 * 1024;
// Retired owners retain half; capture/codec buffers reserve six MiB, leaving
// two MiB for bounded candidate/index bookkeeping and serialization headers.
pub(crate) const MAX_RETIRED_BYTES: usize = MAX_PENDING_WRITE_BYTES / 2;
pub(crate) const ENCODING_BATCH_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CAPTURE_BYTES: usize = MAX_PENDING_WRITE_BYTES / 2 - 2 * 1024 * 1024;

pub struct GameplaySimulationPlugin;
impl Plugin for GameplaySimulationPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(20.0))
            .init_resource::<GameplayClock>()
            .init_resource::<SimulationRegion>()
            .init_resource::<SimulationBudget>()
            .init_resource::<SimulationDiagnostics>()
            .init_resource::<BoundaryWaits>()
            .init_resource::<RandomTickRegistry>()
            .init_resource::<fluids::FluidCatalog>()
            .configure_sets(
                FixedUpdate,
                (
                    SimulationSystems::Activation,
                    SimulationSystems::Neighbors,
                    SimulationSystems::Scheduled,
                    SimulationSystems::Random,
                    SimulationSystems::Machines,
                    SimulationSystems::Actors,
                    SimulationSystems::Finalize,
                )
                    .chain()
                    .run_if(running),
            )
            .configure_sets(
                FixedUpdate,
                (
                    crate::character::CharacterSystems,
                    crate::survival::SurvivalSystems::Prepare,
                    crate::survival::SurvivalSystems::Environment,
                    crate::survival::SurvivalSystems::Consume,
                    crate::survival::SurvivalSystems::Metabolism,
                    crate::survival::SurvivalSystems::Damage,
                )
                    .in_set(SimulationSystems::Actors),
            )
            .add_systems(Startup, cap_virtual_time)
            .add_systems(
                OnEnter(crate::game::GameState::Game),
                restore_clock.after(crate::world::PrepareSession),
            )
            .add_systems(OnExit(crate::game::GameState::Game), clear)
            .add_systems(
                RunFixedMainLoop,
                reset_budget.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(
                FixedUpdate,
                (update_mobile_owners, activate, advance_clock)
                    .chain()
                    .in_set(SimulationSystems::Activation),
            )
            .add_systems(
                FixedUpdate,
                process_neighbors.in_set(SimulationSystems::Neighbors),
            )
            .add_systems(
                FixedUpdate,
                fluids::process_scheduled.in_set(SimulationSystems::Scheduled),
            )
            .add_systems(FixedUpdate, random_ticks.in_set(SimulationSystems::Random))
            .add_systems(
                FixedUpdate,
                update_mobile_owners.in_set(SimulationSystems::Finalize),
            )
            .add_observer(unload)
            .add_observer(remove_work)
            .add_systems(Last, track_work.before(crate::world::WorldSaveCapture));
    }
}
pub(crate) fn running(
    phase: Option<Res<State<crate::world::WorldSessionPhase>>>,
    paused: Option<Res<crate::character::SimulationPaused>>,
) -> bool {
    crate::world::playing(phase) && paused.is_none()
}
fn cap_virtual_time(mut time: ResMut<Time<Virtual>>) {
    time.set_max_delta(Duration::from_millis(100));
}
fn restore_clock(
    mut clock: ResMut<GameplayClock>,
    session: Option<Res<crate::world::WorldSession>>,
) {
    clock.0 = session.map_or(0, |session| session.resume.gameplay_tick);
}
fn clear(
    mut region: ResMut<SimulationRegion>,
    mut waits: ResMut<BoundaryWaits>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut commands: Commands,
) {
    *region = default();
    *waits = default();
    *diagnostics = default();
    commands.remove_resource::<crate::character::SimulationPaused>();
}
fn reset_budget(
    mut budget: ResMut<SimulationBudget>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
) {
    *budget = SimulationBudget {
        neighbors: 4096,
        scheduled: 2048,
        mutations: 4096,
        random: 8192,
    };
    diagnostics.neighbor_cells = 0;
    diagnostics.scheduled_ticks = 0;
    diagnostics.modified_cells = 0;
    diagnostics.random_attempts = 0;
}

// Readiness is re-evaluated only after membership, terrain or ownership changes.
fn activate(
    mut commands: Commands,
    player: Query<&CharacterMotion, With<Player>>,
    settings: Res<SimulationSettings>,
    index: Res<ChunkIndex>,
    mut region: ResMut<SimulationRegion>,
    terrain: Query<(&VoxelChunk, &LoadedColumn)>,
    columns: Query<(
        &GameplayColumn,
        Option<&BlockEntities>,
        Option<&SimulatedEntities>,
        Has<Simulating>,
        &ColumnSimulation,
    )>,
    mut diagnostics: ResMut<SimulationDiagnostics>,
    mut waits: ResMut<BoundaryWaits>,
) {
    let center = player
        .single()
        .ok()
        .map(|motion| chunk_at(motion.position.as_vec3()));
    let radius = settings.effective_distance();
    let changed = region.center != center || region.radius != radius;
    region.center = center;
    region.radius = radius;
    if !changed && !index.is_changed() {
        return;
    }
    let mut active = 0;
    for (chunk, owner) in &terrain {
        let Ok((column, blocks, mobiles, was_active, work)) = columns.get(owner.0) else {
            continue;
        };
        let ready = column.initialized
            && region.contains(chunk.position)
            && (-1..=1).all(|x| {
                (-1..=1).all(|z| index.contains_key(&(chunk.position + IVec2::new(x, z))))
            });
        active += usize::from(ready);
        if was_active == ready {
            continue;
        }
        for entity in std::iter::once(owner.0)
            .chain(blocks.into_iter().flat_map(|entities| entities.iter()))
            .chain(mobiles.into_iter().flat_map(|entities| entities.iter()))
        {
            if ready {
                commands.entity(entity).insert(Simulating);
            } else {
                commands.entity(entity).remove::<Simulating>();
            }
        }
        if ready {
            commands.entity(owner.0).insert(RandomEligibilityDirty);
            waits.wake(chunk.position, &mut commands);
            for task in work.suspended.values() {
                let dependency = IVec2::from_array(task.chunk);
                if let Some(&terrain_entity) = index.get(&dependency)
                    && let Ok((_, target)) = terrain.get(terrain_entity)
                    && columns
                        .get(target.0)
                        .is_ok_and(|(_, _, _, active, _)| active)
                {
                    waits.wake(dependency, &mut commands);
                }
            }
        }
    }
    diagnostics.active_columns = active;
}
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct ReadyTicks;
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct ReadyNeighbors;
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct RandomEligibilityDirty;
fn advance_clock(
    mut commands: Commands,
    mut clock: ResMut<GameplayClock>,
    mut columns: Query<
        (
            Entity,
            &mut ColumnSimulation,
            Has<ReadyTicks>,
            Has<ReadyNeighbors>,
        ),
        With<Simulating>,
    >,
) {
    clock.0 = clock.0.checked_add(1).expect("Gameplay clock exhausted");
    for (entity, mut column, ticks, neighbors) in &mut columns {
        column.local_tick = column
            .local_tick
            .checked_add(1)
            .expect("Column clock exhausted");
        if column.has_due() != ticks {
            if ticks {
                commands.entity(entity).remove::<ReadyTicks>();
            } else {
                commands.entity(entity).insert(ReadyTicks);
            }
        }
        if column.has_neighbors() != neighbors {
            if neighbors {
                commands.entity(entity).remove::<ReadyNeighbors>();
            } else {
                commands.entity(entity).insert(ReadyNeighbors);
            }
        }
    }
}

fn update_mobile_owners(
    mut commands: Commands,
    columns: Res<GameplayColumns>,
    owners: Query<&GameplayColumn>,
    active: Query<(), With<Simulating>>,
    mobiles: Query<
        (
            Entity,
            Option<&CharacterMotion>,
            Option<&crate::dropped_item::DropMotion>,
            Option<&SimulatedIn>,
            Has<Simulating>,
        ),
        Or<(With<CharacterMotion>, With<crate::dropped_item::DropMotion>)>,
    >,
) {
    for (entity, character, drop, previous, was_active) in &mobiles {
        let position = character
            .map(|motion| motion.position)
            .or_else(|| drop.map(|motion| motion.position))
            .expect("Mobile motion");
        let chunk = chunk_at(position.as_vec3());
        let owner = if let Some(previous) = previous
            && owners
                .get(previous.0)
                .is_ok_and(|column| column.position == chunk)
        {
            Some(previous.0)
        } else {
            columns.get(&chunk).copied()
        };
        let changed_owner = owner != previous.map(|previous| previous.0);
        let is_active = owner.is_some_and(|owner| active.contains(owner));
        if !changed_owner && was_active == is_active {
            continue;
        }
        let mut mobile = commands.entity(entity);
        if changed_owner {
            if let Some(owner) = owner {
                mobile.insert(SimulatedIn(owner));
            } else {
                mobile.remove::<SimulatedIn>();
            }
        }
        if is_active {
            mobile.insert(Simulating);
        } else {
            mobile.remove::<Simulating>();
        }
    }
}

fn track_work(
    mut columns: Query<(&mut GameplayColumn, Ref<ColumnSimulation>)>,
    drops: Query<
        &SimulatedIn,
        Or<(
            Changed<crate::dropped_item::DroppedItem>,
            Changed<crate::dropped_item::DropMotion>,
        )>,
    >,
) {
    for (mut column, work) in &mut columns {
        if work.is_changed() {
            column.revision += 1;
        }
    }
    for owner in &drops {
        if let Ok((mut column, _)) = columns.get_mut(owner.0) {
            column.revision += 1;
        }
    }
}
fn unload(
    event: On<Remove<VoxelChunk>>,
    terrain: Query<&LoadedColumn>,
    mut columns: Query<(
        &mut ColumnSimulation,
        Option<&SimulatedEntities>,
        Option<&BlockEntities>,
    )>,
    drops: Query<(
        &crate::dropped_item::DroppedItem,
        &crate::dropped_item::DropMotion,
    )>,
    mut commands: Commands,
) {
    let Ok(owner) = terrain.get(event.entity) else {
        return;
    };
    commands
        .entity(owner.0)
        .remove::<(Simulating, crate::voxel::streaming::RenderRequested)>();
    let Ok((mut work, mobiles, blocks)) = columns.get_mut(owner.0) else {
        return;
    };
    if let Some(blocks) = blocks {
        for entity in blocks.iter() {
            commands.entity(entity).remove::<Simulating>();
        }
    }
    if let Some(mobiles) = mobiles {
        for entity in mobiles.iter() {
            if let Ok((drop, motion)) = drops.get(entity) {
                work.dormant_drops.push(drop.capture(motion));
                commands.entity(entity).despawn();
            } else {
                commands
                    .entity(entity)
                    .remove::<(SimulatedIn, Simulating)>();
            }
        }
    }
}
fn remove_work(event: On<Remove<ColumnSimulation>>, mut waits: ResMut<BoundaryWaits>) {
    waits.remove_owner(event.entity);
}

/// Conservative retained bytes, counting complete shared section allocations.
pub(crate) fn terrain_bytes(voxels: &crate::voxel::ChunkVoxels) -> usize {
    use hollow_core::sections::Section;
    let headers = std::mem::size_of::<crate::voxel::ChunkVoxels>()
        + hollow_core::coordinates::SECTION_COUNT
            * (std::mem::size_of::<Section<crate::voxel::VoxelState>>()
                + 2 * std::mem::size_of::<Section<u8>>())
        + 48;
    headers
        + (0..hollow_core::coordinates::SECTION_COUNT)
            .map(|section| {
                let states = match voxels.states.section(section) {
                    Section::Uniform(_) => 0,
                    Section::Dense(_) => 4096 * std::mem::size_of::<crate::voxel::VoxelState>(),
                };
                // Also reserve a possible rendering-light estimate layer.
                let light = match voxels.light.section(section) {
                    Section::Uniform(_) => 0,
                    Section::Dense(_) => 4096 * 2,
                };
                states + light
            })
            .sum::<usize>()
}
