//! Structured readiness of a streamed area, without application loading text.
use crate::build::{ChunkBuild, ChunkSections, NeedsChunkBuild};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_core::{VoxelCell, coordinates::chunk_at};
use hollow_engine::{
    blocks::LoadedColumn,
    chunk::ChunkVoxels,
    simulation::ColumnData,
    streaming::{AwaitingNeighbors, StreamOffsets, StreamState},
    world::ChunkIndex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Demand,
    Terrain,
    Meshes,
    Ready,
}

#[derive(SystemParam)]
pub struct WorldReadiness<'w, 's, T: VoxelCell, A: Send + Sync + 'static> {
    index: Res<'w, ChunkIndex>,
    offsets: Res<'w, StreamOffsets>,
    stream: Res<'w, StreamState>,
    chunks: Query<'w, 's, (&'static ChunkVoxels<T>, &'static LoadedColumn)>,
    gameplay: Query<'w, 's, &'static ColumnData<A>>,
    rendered: Query<
        'w,
        's,
        (),
        (
            With<ChunkSections>,
            Without<AwaitingNeighbors>,
            Without<NeedsChunkBuild>,
            Without<ChunkBuild>,
        ),
    >,
}

impl<T: VoxelCell, A: Send + Sync + 'static> WorldReadiness<'_, '_, T, A> {
    pub fn progress(&self, position: Vec3) -> Readiness {
        let center = chunk_at(position);
        if self.stream.center != Some(center) {
            return Readiness::Demand;
        }
        let render_squared = (self.offsets.render_radius * self.offsets.render_radius) as i32;
        let (mut terrain, mut meshes) = (true, true);
        for &position in &self.stream.data {
            let offset = position - center;
            let Some(&entity) = self.index.get(&position) else {
                terrain = false;
                continue;
            };
            let Ok((voxels, owner)) = self.chunks.get(entity) else {
                terrain = false;
                continue;
            };
            if !self
                .gameplay
                .get(owner.0)
                .is_ok_and(|column| column.initialized)
            {
                terrain = false;
            }
            if offset.length_squared() <= render_squared
                && (voxels.dirty_sections != 0 || !self.rendered.contains(entity))
            {
                meshes = false;
            }
        }
        if self.stream.data.is_empty() || !terrain {
            Readiness::Terrain
        } else if !meshes {
            Readiness::Meshes
        } else {
            Readiness::Ready
        }
    }
}
