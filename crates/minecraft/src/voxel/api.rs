use super::{SetVoxel, VoxelKind, VoxelState};
use bevy::prelude::*;
pub type VoxelWorld<'w, 's> = hollow_engine::world::VoxelWorld<'w, 's, VoxelState>;

pub trait VoxelCommandsExt {
    /// Queues a world-space voxel edit, generating an unloaded chunk when necessary.
    fn set_voxel(&mut self, world_position: IVec3, kind: VoxelKind) -> &mut Self;
    fn set_voxel_state(&mut self, world_position: IVec3, state: VoxelState) -> &mut Self;
}

impl VoxelCommandsExt for Commands<'_, '_> {
    fn set_voxel_state(&mut self, world_position: IVec3, state: VoxelState) -> &mut Self {
        self.trigger(SetVoxel::with_state(world_position, state));
        self
    }
    fn set_voxel(&mut self, world_position: IVec3, kind: VoxelKind) -> &mut Self {
        self.trigger(SetVoxel::new(world_position, kind));
        self
    }
}
