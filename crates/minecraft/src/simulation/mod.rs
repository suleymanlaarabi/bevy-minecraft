//! Fixed gameplay time, spatial activation and shared per-frame work limits.
use hollow_features::motion::FixedPosition;
mod work;
pub(crate) use work::*;

use crate::{
    block_behavior::containers::{BlockEntities, GameplayColumn, GameplayColumns, LoadedColumn},
    player::Player,
    settings::SimulationSettings,
    voxel::{VoxelChunk, chunk_at},
};
use bevy::{ecs::query::QueryFilter, prelude::*};
use std::time::Duration;

pub use hollow_engine::simulation::*;
pub(crate) const MAX_PENDING_WRITE_BYTES: usize = 16 * 1024 * 1024;
// Retired owners retain half; capture/codec buffers reserve six MiB, leaving
// two MiB for bounded candidate/index bookkeeping and serialization headers.
pub(crate) const MAX_RETIRED_BYTES: usize = MAX_PENDING_WRITE_BYTES / 2;
pub(crate) const ENCODING_BATCH_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CAPTURE_BYTES: usize = MAX_PENDING_WRITE_BYTES / 2 - 2 * 1024 * 1024;

pub struct GameplaySimulationPlugin;
impl Plugin for GameplaySimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(hollow_engine::simulation::runtime::VoxelSimulationPlugin::<
            MinecraftRules,
        >::default())
            .insert_resource(MinecraftRules)
            .insert_resource(Time::<Fixed>::from_hz(20.0))
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
            .add_systems(Startup, cap_virtual_time)
            .add_systems(
                OnEnter(crate::game::GameState::Game),
                restore_clock.after(crate::world::PrepareSession),
            )
            .add_systems(
                FixedUpdate,
                (sync_region, update_mobile_owners::<()>)
                    .chain()
                    .in_set(SimulationSystems::Activation)
                    .before(
                        hollow_engine::simulation::runtime::activate::<
                            crate::creatures::ColumnActors,
                        >,
                    ),
            )
            .add_systems(
                FixedUpdate,
                update_mobile_owners::<Changed<FixedPosition>>.in_set(SimulationSystems::Finalize),
            )
            .add_observer(displaced)
            .add_observer(unload)
            .add_systems(Last, track_work.before(crate::world::WorldSaveCapture));
    }
}
pub(crate) fn running(
    phase: Option<Res<State<crate::world::WorldSessionPhase>>>,
    paused: Option<Res<crate::simulation::SimulationPaused>>,
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
fn sync_region(
    player: Query<&FixedPosition, With<Player>>,
    settings: Res<SimulationSettings>,
    voxel: Res<crate::voxel::VoxelSettings>,
    mut region: ResMut<SimulationRegion>,
    mut seed: ResMut<hollow_engine::simulation::SimulationSeed>,
) {
    region.set_if_neq(SimulationRegion {
        center: player
            .single()
            .ok()
            .map(|motion| chunk_at(motion.current.as_vec3())),
        radius: settings.effective_distance(),
    });
    seed.0 = voxel.seed;
}

fn update_mobile_owners<F: QueryFilter>(
    mut commands: Commands,
    columns: Res<GameplayColumns>,
    mut owners: Query<&mut GameplayColumn>,
    active: Query<(), With<Simulating>>,
    mobiles: Query<
        (
            Entity,
            &FixedPosition,
            Option<&SimulatedIn>,
            Has<Simulating>,
            Has<crate::creatures::Creature>,
        ),
        F,
    >,
) {
    for (entity, position, previous, was_active, creature) in &mobiles {
        let chunk = chunk_at(position.current.as_vec3());
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
        if changed_owner && creature {
            for owner in [previous.map(|p| p.0), owner].into_iter().flatten() {
                if let Ok(mut column) = owners.get_mut(owner) {
                    column.revision += 1;
                }
            }
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
    // Sequential access: the filtered pass visits only changed work, then drops
    // resolve their owners.
    mut columns: ParamSet<(
        Query<&mut GameplayColumn, Changed<ColumnSimulation>>,
        Query<&mut GameplayColumn>,
    )>,
    drops: Query<
        &SimulatedIn,
        (
            With<crate::dropped_item::DroppedItem>,
            Or<(
                Changed<crate::dropped_item::DroppedItem>,
                Changed<FixedPosition>,
            )>,
        ),
    >,
) {
    for mut column in &mut columns.p0() {
        column.revision += 1;
    }
    let mut columns = columns.p1();
    for owner in &drops {
        if let Ok(mut column) = columns.get_mut(owner.0) {
            column.revision += 1;
        }
    }
}
fn unload(
    event: On<Remove<VoxelChunk>>,
    terrain: Query<&LoadedColumn>,
    mut columns: Query<(
        &mut GameplayColumn,
        Option<&SimulatedEntities>,
        Option<&BlockEntities>,
    )>,
    drops: Query<(
        &crate::dropped_item::DroppedItem,
        &crate::dropped_item::DropMotion,
        &FixedPosition,
    )>,
    creatures: crate::creatures::CreatureSnapshots,
    creature_entities: Query<(), With<crate::creatures::Creature>>,
    mut commands: Commands,
) {
    let Ok(owner) = terrain.get(event.entity) else {
        return;
    };
    commands
        .entity(owner.0)
        .remove::<(Simulating, hollow_render::streaming::RenderRequested)>();
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
            if let Ok((drop, motion, position)) = drops.get(entity) {
                work.application.drops.push(drop.capture(motion, position));
                commands.entity(entity).despawn();
            } else if let Some(saved) = creatures.capture(entity) {
                work.application.creatures.push(saved);
                work.revision += 1;
                commands.entity(entity).despawn();
            } else if creature_entities.contains(entity) {
                commands.entity(entity).despawn();
            } else {
                commands
                    .entity(entity)
                    .remove::<(SimulatedIn, Simulating)>();
            }
        }
    }
}
