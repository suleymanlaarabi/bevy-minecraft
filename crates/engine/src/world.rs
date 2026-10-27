//! Read-only voxel access; each system declares its terrain borrow explicitly.
use crate::chunk::ChunkVoxels;
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::{prelude::*, system::SystemParam};
use bevy_math::prelude::*;
use bevy_platform::collections::HashMap;
use hollow_core::{
    VoxelCell,
    coordinates::{chunk_at, split_world_position},
};

/// Coordinates are sparse and unbounded; an indexed dense world table is unsuitable.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct ChunkIndex(HashMap<IVec2, Entity>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldSample<T> {
    Available(T),
    Unavailable,
    OutsideBounds,
}

#[derive(SystemParam)]
pub struct VoxelWorld<'w, 's, T: VoxelCell> {
    index: Res<'w, ChunkIndex>,
    chunks: Query<'w, 's, &'static ChunkVoxels<T>>,
}

impl<T: VoxelCell> VoxelWorld<'_, '_, T> {
    /// Whether the voxels around a position are loaded. Character movement reads
    /// them directly without building a separate collision representation.
    pub fn collision_ready(&self, position: Vec3) -> bool {
        self.index
            .get(&chunk_at(position))
            .is_some_and(|entity| self.chunks.contains(*entity))
    }

    /// Local collision cache key; unloading and reloading also changes the entity.
    pub fn column_revision(&self, position: IVec2) -> Option<(Entity, u64)> {
        let entity = *self.index.get(&position)?;
        Some((entity, self.chunks.get(entity).ok()?.content_revision))
    }

    /// Visits every cell of an inclusive region, each column from top to bottom,
    /// looking its chunk up once. Cells of unloaded chunks come back as `None`;
    /// cells above or below the world are air.
    pub fn visit_cells(&self, min: IVec3, max: IVec3, mut visit: impl FnMut(IVec3, Option<T>)) {
        let air = T::air();
        for x in min.x..=max.x {
            for z in min.z..=max.z {
                let (chunk, local) = split_world_position(IVec3::new(x, 0, z));
                let voxels = self
                    .index
                    .get(&chunk)
                    .and_then(|&entity| self.chunks.get(entity).ok());
                for y in (min.y..=max.y).rev() {
                    let cell = IVec3::new(x, y, z);
                    visit(
                        cell,
                        voxels.map(|voxels| voxels.state(local.with_y(y)).unwrap_or(air)),
                    );
                }
            }
        }
    }

    /// Distinguishes missing terrain from positions outside the column bounds.
    pub fn sample(&self, position: IVec3) -> WorldSample<T> {
        if !(0..hollow_core::coordinates::CHUNK_HEIGHT).contains(&position.y) {
            WorldSample::OutsideBounds
        } else {
            self.state(position)
                .map_or(WorldSample::Unavailable, WorldSample::Available)
        }
    }

    /// Returns the voxel at an integer world position, or `None` when it is unavailable.
    pub fn get(&self, world_position: IVec3) -> Option<T::Kind> {
        self.state(world_position).map(T::kind)
    }

    pub fn state(&self, world_position: IVec3) -> Option<T> {
        let (chunk, local) = split_world_position(world_position);
        self.index
            .get(&chunk)
            .and_then(|&entity| self.chunks.get(entity).ok())
            .and_then(|voxels| voxels.state(local))
    }

    pub fn light(&self, position: IVec3) -> Option<u8> {
        let (chunk, local) = split_world_position(position);
        self.chunks
            .get(*self.index.get(&chunk)?)
            .ok()?
            .render_light_at(local)
    }

    pub fn is_liquid_at(&self, position: Vec3) -> bool {
        self.state(position.floor().as_ivec3())
            .is_some_and(T::is_liquid)
    }

    /// Returns the voxel containing a continuous world position.
    pub fn get_at(&self, world_position: Vec3) -> Option<T::Kind> {
        self.get(world_position.floor().as_ivec3())
    }
}
