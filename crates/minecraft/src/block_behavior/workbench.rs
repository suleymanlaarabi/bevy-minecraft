//! Workbenches compose the shared inventory station protocol without owning slots.
use bevy::prelude::*;

pub struct WorkbenchPlugin;
impl Plugin for WorkbenchPlugin {
    fn build(&self, app: &mut App) {
        let kind = crate::voxel::VoxelKind::from_native(pumpkin_data::BlockId::CRAFTING_TABLE);
        app.world_mut()
            .resource_mut::<bevy_voxel::engine::blocks::BlockRegistrations>()
            .register_bundle::<crate::inventory::InventoryStation>(
                bevy_voxel::core::BlockId(kind.index() as u32),
                true,
            );
    }
}
