//! Read-only voxel access; each system declares its terrain borrow explicitly.
use crate::chunk::ChunkVoxels;
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::{prelude::*, system::SystemParam};
use bevy_math::prelude::*;
use bevy_platform::collections::HashMap;
use hollow_core::{
    VoxelCell,
    coordinates::{ColumnLayout, chunk_at},
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

#[derive(Resource, Default, Clone, Copy, Deref)]
pub struct WorldLayout(pub ColumnLayout);

#[derive(SystemParam)]
pub struct VoxelWorld<'w, 's, T: VoxelCell> {
    content: Res<'w, crate::content::RegisteredContent>,
    layout: Res<'w, WorldLayout>,
    index: Res<'w, ChunkIndex>,
    chunks: Query<'w, 's, &'static ChunkVoxels<T>>,
}

impl<T: VoxelCell> VoxelWorld<'_, '_, T> {
    pub fn layout(&self) -> hollow_core::coordinates::ColumnLayout {
        self.layout.0
    }

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

    /// Visits every cell of an inclusive region, each vertical stack top to bottom.
    /// Small regions resolve each chunk once. Cells of unloaded chunks come back as
    /// `None`; cells above or below the world are air.
    pub fn visit_cells(&self, min: IVec3, max: IVec3, mut visit: impl FnMut(IVec3, Option<T>)) {
        let air = T::air();
        let first = self.layout.split(min).0;
        let last = self.layout.split(max).0;
        let width = i64::from(last.x) - i64::from(first.x) + 1;
        let depth = i64::from(last.y) - i64::from(first.y) + 1;
        // Character probes normally touch at most four columns. Resolve those once
        // on the stack; large regions keep the bounded strip-based fallback.
        let small_region = width > 0 && depth > 0 && width * depth <= 4;
        let mut nearby = [None; 4];
        if small_region {
            for x in first.x..=last.x {
                for z in first.y..=last.y {
                    let slot = ((i64::from(x) - i64::from(first.x)) * depth + i64::from(z)
                        - i64::from(first.y)) as usize;
                    nearby[slot] = self
                        .index
                        .get(&IVec2::new(x, z))
                        .and_then(|&entity| self.chunks.get(entity).ok());
                }
            }
        }
        for x in min.x..=max.x {
            let mut cached_chunk = None;
            let mut cached_voxels = None;
            for z in min.z..=max.z {
                let (chunk, local) = self.layout.split(IVec3::new(x, 0, z));
                if small_region {
                    let slot = ((i64::from(chunk.x) - i64::from(first.x)) * depth
                        + i64::from(chunk.y)
                        - i64::from(first.y)) as usize;
                    cached_voxels = nearby[slot];
                } else if cached_chunk != Some(chunk) {
                    cached_chunk = Some(chunk);
                    cached_voxels = self
                        .index
                        .get(&chunk)
                        .and_then(|&entity| self.chunks.get(entity).ok());
                }
                for y in (min.y..=max.y).rev() {
                    let cell = IVec3::new(x, y, z);
                    visit(
                        cell,
                        cached_voxels.map(|voxels| {
                            voxels
                                .state(local.with_y(y.saturating_sub(self.layout.min_y())))
                                .unwrap_or(air)
                        }),
                    );
                }
            }
        }
    }

    /// Distinguishes missing terrain from positions outside the column bounds.
    pub fn sample(&self, position: IVec3) -> WorldSample<T> {
        if !self.layout.contains(position.y) {
            WorldSample::OutsideBounds
        } else {
            self.state(position)
                .map_or(WorldSample::Unavailable, WorldSample::Available)
        }
    }

    /// Returns the voxel at an integer world position, or `None` when it is unavailable.
    pub fn get(&self, world_position: IVec3) -> Option<T::Kind> {
        self.state(world_position)
            .map(|state| state.kind(&self.content))
    }

    pub fn state(&self, world_position: IVec3) -> Option<T> {
        let (chunk, local) = self.layout.split(world_position);
        self.index
            .get(&chunk)
            .and_then(|&entity| self.chunks.get(entity).ok())
            .and_then(|voxels| voxels.state(local))
    }

    pub fn gameplay_light(&self, position: IVec3) -> Option<u8> {
        let (chunk, local) = self.layout.split(position);
        self.chunks
            .get(*self.index.get(&chunk)?)
            .ok()?
            .gameplay_light_at(local)
    }

    pub fn light(&self, position: IVec3) -> Option<u8> {
        let (chunk, local) = self.layout.split(position);
        self.chunks
            .get(*self.index.get(&chunk)?)
            .ok()?
            .render_light_at(local)
    }

    pub fn is_liquid_at(&self, position: Vec3) -> bool {
        self.state(position.floor().as_ivec3())
            .is_some_and(|state| state.is_liquid(&self.content))
    }

    /// Returns the voxel containing a continuous world position.
    pub fn get_at(&self, world_position: Vec3) -> Option<T::Kind> {
        self.get(world_position.floor().as_ivec3())
    }
}
