//! Sparse ECS gameplay over compact voxel storage.
pub(crate) mod catalog;
pub(crate) mod containers;
pub(crate) mod interaction;
pub(crate) mod loot;
pub(crate) mod supports;
pub(crate) mod visuals;
pub(crate) mod workbench;
use bevy::prelude::*;
pub(crate) use containers::ContainerInventory;
pub use containers::{BlockEntity, BlockState, ChestPlugin};
pub use interaction::{BlockUse, ManualOpeningPlugin};
pub use workbench::WorkbenchPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GameplaySystems {
    Topology,
    Validation,
    Inventory,
    Derived,
    Presentation,
}

/// Minecraft content is a composition of independently registered behaviors.
pub struct BlockBehaviorPlugins;
impl PluginGroup for BlockBehaviorPlugins {
    fn build(self) -> bevy::app::PluginGroupBuilder {
        bevy::app::PluginGroupBuilder::start::<Self>()
            .add(hollow_engine::blocks::BlockRegistryPlugin)
            .add(hollow_features::articulation::ArticulationPlugin)
            .add(BlockLifecyclePlugin)
            .add(containers::ChestPlugin)
            .add(workbench::WorkbenchPlugin)
            .add(interaction::ManualOpeningPlugin)
    }
}

pub struct BlockLifecyclePlugin;
impl Plugin for BlockLifecyclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<containers::GameplayColumns>()
            .init_resource::<loot::LootRandom>()
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
            .add_systems(
                PreStartup,
                (catalog::initialize, loot::initialize)
                    .after(hollow_engine::blocks::FinalizeBlockContent),
            )
            .add_observer(containers::changed)
            .add_observer(containers::loaded)
            .add_observer(containers::unload);
    }
}
