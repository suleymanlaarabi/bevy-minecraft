//! Readiness of the streamed spawn area before the player enters the world.
use hollow_render::build::{ChunkBuild, ChunkSections, NeedsChunkBuild};

use super::{
    ChunkVoxels,
    streaming::{AwaitingNeighbors, ChunkIndex, StreamOffsets},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_core::coordinates::chunk_at;

#[derive(SystemParam)]
pub(crate) struct VoxelLoading<'w, 's> {
    index: Res<'w, ChunkIndex>,
    offsets: Res<'w, StreamOffsets>,
    stream: Res<'w, super::streaming::StreamState>,
    chunks: Query<
        'w,
        's,
        (
            &'static ChunkVoxels,
            &'static crate::block_behavior::containers::LoadedColumn,
        ),
    >,
    gameplay: Query<'w, 's, &'static crate::block_behavior::containers::GameplayColumn>,
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

impl VoxelLoading<'_, '_> {
    pub(crate) fn progress(&self, position: Vec3) -> (bool, &'static str) {
        let center = chunk_at(position);
        if self.stream.center != Some(center) {
            return (false, "Preparing terrain demand…");
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
            (false, "Loading and generating terrain…")
        } else if !meshes {
            (false, "Building terrain meshes…")
        } else {
            (true, "Ready")
        }
    }
}
