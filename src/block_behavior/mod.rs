//! Sparse ECS gameplay over compact voxel storage.
pub(crate) mod catalog;
pub(crate) mod containers;
pub(crate) mod interaction;
mod queue;
pub(crate) mod supports;
pub(crate) mod visuals;
use bevy::prelude::*;
pub(crate) use containers::ContainerInventory;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GameplaySystems {
    Topology,
    Validation,
    Inventory,
    Derived,
    Presentation,
}

pub struct BlockBehaviorPlugin;
impl Plugin for BlockBehaviorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<containers::GameplayColumns>()
            .init_resource::<supports::SupportQueue>()
            .init_resource::<visuals::ChestMeshes>()
            .configure_sets(
                PostUpdate,
                (
                    GameplaySystems::Topology.after(crate::voxel::VoxelPipeline),
                    GameplaySystems::Validation,
                    GameplaySystems::Inventory,
                    GameplaySystems::Derived,
                    GameplaySystems::Presentation,
                )
                    .chain(),
            )
            .add_systems(PreStartup, catalog::initialize)
            .add_systems(
                OnEnter(crate::game::GameState::Game),
                supports::restore.after(crate::world::PrepareSession),
            )
            .add_observer(containers::changed)
            .add_observer(containers::unload)
            .add_observer(containers::viewing_closed)
            .add_observer(containers::pairing_removed)
            .add_observer(supports::changed)
            .add_observer(supports::arrived)
            .add_observer(interaction::open_chest)
            .add_observer(interaction::open_manual)
            .add_systems(Update, supports::update.run_if(crate::world::playing))
            .add_systems(
                PostUpdate,
                containers::update_topology
                    .in_set(GameplaySystems::Topology)
                    .run_if(crate::world::streaming),
            )
            .add_systems(
                PostUpdate,
                containers::validate_sessions
                    .in_set(GameplaySystems::Validation)
                    .run_if(crate::world::playing),
            )
            .add_systems(
                OnExit(crate::inventory::InventoryState::Open),
                containers::close_sessions,
            )
            .add_systems(OnExit(crate::game::GameState::Game), cleanup);
        visuals::register(app);
    }
}
fn cleanup(
    mut columns: ResMut<containers::GameplayColumns>,
    mut supports: ResMut<supports::SupportQueue>,
) {
    columns.clear();
    *supports = default();
}
