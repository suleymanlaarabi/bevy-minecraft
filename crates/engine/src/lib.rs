//! Content-independent Bevy services for voxel applications.
pub mod blocks;
pub mod chunk;
pub mod edits;
pub mod work;
pub mod world;

use bevy_app::{App, Plugin};
use hollow_core::VoxelCell;
use std::marker::PhantomData;

/// A terrain application supplies its cell semantics and generation/streaming policy.
pub struct VoxelWorldPlugin<T: VoxelCell>(PhantomData<T>);
impl<T: VoxelCell> Default for VoxelWorldPlugin<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<T: VoxelCell> Plugin for VoxelWorldPlugin<T> {
    fn build(&self, app: &mut App) {
        app.init_resource::<world::ChunkIndex>()
            .init_resource::<edits::StoredChunks<T>>()
            .init_resource::<edits::PendingEdits<T>>()
            .add_observer(edits::set_voxel::<T>);
    }
}
pub mod raycast;

pub mod content;

pub mod fluids;
