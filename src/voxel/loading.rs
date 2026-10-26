//! Readiness of the streamed spawn area before the player enters the world.

use super::{
    ChunkVoxels,
    render::build::{ChunkBuild, ChunkSections, NeedsChunkBuild},
    streaming::{AwaitingNeighbors, ChunkIndex, StreamOffsets},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_worldgen::coordinates::chunk_at;

#[derive(SystemParam)]
pub(crate) struct VoxelLoading<'w, 's> {
    index: Res<'w, ChunkIndex>,
    offsets: Res<'w, StreamOffsets>,
    chunks: Query<'w, 's, &'static ChunkVoxels>,
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
        let render_squared = (self.offsets.render_radius * self.offsets.render_radius) as i32;
        let (mut terrain, mut meshes) = (true, true);
        for &offset in self.offsets.iter() {
            let Some(&entity) = self.index.get(&(center + offset)) else {
                terrain = false;
                continue;
            };
            let Ok(voxels) = self.chunks.get(entity) else {
                terrain = false;
                continue;
            };
            if offset.length_squared() <= render_squared
                && (voxels.dirty_sections != 0 || !self.rendered.contains(entity))
            {
                meshes = false;
            }
        }
        if !terrain {
            (false, "Loading and generating terrain…")
        } else if !meshes {
            (false, "Building terrain meshes…")
        } else {
            (true, "Ready")
        }
    }
}
