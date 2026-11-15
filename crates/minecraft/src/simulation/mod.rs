//! Fixed gameplay time, spatial activation and shared per-frame work limits.
use bevy_voxel::features::motion::FixedPosition;
mod work;
pub(crate) use work::*;

use crate::{
    block_behavior::containers::{BlockEntities, GameplayColumn, GameplayColumns, LoadedColumn},
    player::Player,
    settings::SimulationSettings,
    voxel::{VoxelChunk, chunk_at},
};
use bevy::prelude::*;
use bevy_voxel::spatial::update_mobile_owners;
use std::time::Duration;

pub use bevy_voxel::engine::simulation::*;
pub(crate) const MAX_PENDING_WRITE_BYTES: usize = 16 * 1024 * 1024;
// Retired owners retain half; capture/codec buffers reserve six MiB, leaving
// two MiB for bounded candidate/index bookkeeping and serialization headers.
pub(crate) const MAX_RETIRED_BYTES: usize = MAX_PENDING_WRITE_BYTES / 2;
pub(crate) const ENCODING_BATCH_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_CAPTURE_BYTES: usize = MAX_PENDING_WRITE_BYTES / 2 - 2 * 1024 * 1024;

pub struct GameplaySimulationPlugin;
impl Plugin for GameplaySimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_voxel::engine::simulation::runtime::VoxelSimulationPlugin::<
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
                (
                    sync_region,
                    update_mobile_owners::<crate::creatures::ColumnActors, crate::creatures::Creature, ()>.run_if(resource_changed::<GameplayColumns>),
                    update_mobile_owners::<crate::creatures::ColumnActors, crate::creatures::Creature, Changed<FixedPosition>>
                        .run_if(not(resource_changed::<GameplayColumns>)),
                )
                    .chain()
                    .in_set(SimulationSystems::Activation)
                    .before(
                        bevy_voxel::engine::simulation::runtime::activate::<
                            crate::creatures::ColumnActors,
                        >,
                    ),
            )
            .add_systems(
                FixedUpdate,
                update_mobile_owners::<crate::creatures::ColumnActors, crate::creatures::Creature, Changed<FixedPosition>>.in_set(SimulationSystems::Finalize),
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
    mut seed: ResMut<bevy_voxel::engine::simulation::SimulationSeed>,
) {
    region.set_if_neq(SimulationRegion {
        center: player
            .single()
            .ok()
            .map(|motion| chunk_at(motion.current.as_vec3())),
        radius: settings.effective_distance(),
    });
    if seed.0 != voxel.seed {
        seed.0 = voxel.seed;
    }
}

fn track_work(
    work: Query<Entity, Changed<ColumnSimulation>>,
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
    mut columns: Query<&mut GameplayColumn>,
    mut changed: Local<Vec<Entity>>,
) {
    changed.clear();
    changed.extend(work.iter());
    changed.extend(drops.iter().map(|owner| owner.0));
    changed.sort_unstable();
    changed.dedup();
    for &owner in changed.iter() {
        if let Ok(mut column) = columns.get_mut(owner) {
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
        .remove::<(Simulating, bevy_voxel::render::streaming::RenderRequested)>();
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
