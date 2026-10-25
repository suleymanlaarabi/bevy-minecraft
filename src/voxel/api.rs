use super::{
    ChunkVoxels, SetVoxel, VoxelKind, VoxelState, chunk_at, split_world_position,
    streaming::ChunkIndex,
};
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(SystemParam)]
pub struct VoxelWorld<'w, 's> {
    index: Res<'w, ChunkIndex>,
    chunks: Query<'w, 's, &'static ChunkVoxels>,
    colliders: Query<'w, 's, (), With<avian3d::prelude::Collider>>,
}

impl VoxelWorld<'_, '_> {
    /// Collision is installed after asynchronous terrain/mesh generation.
    pub fn collision_ready(&self, position: Vec3) -> bool {
        self.index
            .get(&chunk_at(position))
            .is_some_and(|entity| self.colliders.contains(*entity))
    }

    /// Covers the body's current footprint and its next fixed-step footprint.
    pub(crate) fn collision_ready_for_motion(
        &self,
        position: Vec3,
        velocity: Vec3,
        dt: f32,
    ) -> bool {
        let end = position + velocity * dt;
        let min = position.min(end) - Vec3::splat(0.35);
        let max = position.max(end) + Vec3::splat(0.35);
        [
            Vec3::new(min.x, position.y, min.z),
            Vec3::new(min.x, position.y, max.z),
            Vec3::new(max.x, position.y, min.z),
            Vec3::new(max.x, position.y, max.z),
        ]
        .into_iter()
        .all(|p| self.collision_ready(p))
    }

    /// Returns the voxel at an integer world position, or `None` when it is unavailable.
    pub fn get(&self, world_position: IVec3) -> Option<VoxelKind> {
        let (chunk_position, local_position) = split_world_position(world_position);
        self.index
            .get(&chunk_position)
            .and_then(|entity| self.chunks.get(*entity).ok())
            .and_then(|voxels| voxels.get(local_position))
    }

    pub fn state(&self, world_position: IVec3) -> Option<VoxelState> {
        let (chunk, local) = split_world_position(world_position);
        self.index
            .get(&chunk)
            .and_then(|&entity| self.chunks.get(entity).ok())
            .and_then(|voxels| voxels.state(local))
    }

    pub fn is_liquid_at(&self, position: Vec3) -> bool {
        self.state(position.floor().as_ivec3())
            .is_some_and(|state| state.has_water() || state.kind() == VoxelKind::Lava)
    }

    /// Returns the voxel containing a continuous world position.
    pub fn get_at(&self, world_position: Vec3) -> Option<VoxelKind> {
        self.get(world_position.floor().as_ivec3())
    }
}

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

#[cfg(test)]
mod tests {

    use super::*;
    use avian3d::prelude::Collider;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn collision_readiness_waits_for_the_loaded_chunk_collider() {
        let mut world = World::new();
        world.init_resource::<ChunkIndex>();
        let chunk = world.spawn_empty().id();
        world
            .resource_mut::<ChunkIndex>()
            .insert(IVec2::ZERO, chunk);
        let ready = |voxels: VoxelWorld| voxels.collision_ready(Vec3::new(8.0, 130.0, 8.0));
        assert!(!world.run_system_once(ready).unwrap());
        world
            .entity_mut(chunk)
            .insert(Collider::cuboid(16.0, 1.0, 16.0));
        assert!(world.run_system_once(ready).unwrap());
        world.despawn(chunk);
        assert!(!world.run_system_once(ready).unwrap());
    }
}
