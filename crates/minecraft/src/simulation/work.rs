//! Minecraft rules plugged into the shared voxel scheduler.
use crate::voxel::{
    VoxelState,
    edits::{CheckedEdit, EditBatch},
};
use bevy::prelude::*;
#[derive(Resource)]
pub(crate) struct MinecraftRules;
impl hollow_engine::simulation::runtime::NeighborRules for MinecraftRules {
    type Cell = VoxelState;
    type Application = Vec<crate::world::SavedDrop>;
    type Support = EditBatch;
    fn connected(
        &self,
        position: IVec3,
        state: VoxelState,
        sample: impl FnMut(IVec3) -> Option<VoxelState>,
    ) -> VoxelState {
        crate::voxel::rules::connected(position, state, sample)
    }
    fn support(
        &self,
        position: IVec3,
        state: VoxelState,
        sample: impl FnMut(IVec3) -> Option<VoxelState>,
    ) -> Result<Option<EditBatch>, IVec2> {
        crate::block_behavior::supports::evaluate(position, state, sample)
    }
    fn edits(edit: &EditBatch) -> &[CheckedEdit] {
        &edit.edits
    }
}
pub(crate) type NeighborUpdates<'w, 's> =
    hollow_engine::simulation::runtime::NeighborUpdates<'w, 's, VoxelState>;
pub fn displaced(
    event: On<hollow_engine::simulation::runtime::FluidDisplaced<VoxelState>>,
    mut commands: Commands,
    mut random: ResMut<crate::block_behavior::loot::LootRandom>,
) {
    use crate::dropped_item::DroppedItemCommandsExt;
    for stack in crate::block_behavior::loot::drops(event.state, false, &mut random)
        .into_iter()
        .flatten()
    {
        commands.spawn_dropped_item(
            stack,
            event.position.as_vec3() + Vec3::splat(0.5),
            Vec3::Y * 1.2,
        );
    }
}
