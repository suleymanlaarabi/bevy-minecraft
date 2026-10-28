//! Content-independent Bevy services for voxel applications.
pub mod blocks;
pub mod chunk;
pub mod content;
pub mod edits;
pub mod fluids;
pub mod lighting;
pub mod raycast;
pub mod session;
pub mod simulation;
pub mod storage;
pub mod streaming;
pub mod tasks;
pub mod work;
pub mod world;

use bevy_app::{App, Plugin};
use hollow_core::VoxelCell;
use std::marker::PhantomData;

pub struct VoxelWorldPlugin<T: VoxelCell>(PhantomData<T>);
impl<T: VoxelCell> Default for VoxelWorldPlugin<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<T: VoxelCell> Plugin for VoxelWorldPlugin<T> {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<content::ContentPlugin>() {
            app.add_plugins(content::ContentPlugin);
        }
        if !app.is_plugin_added::<tasks::VoxelTaskPlugin>() {
            app.add_plugins(tasks::VoxelTaskPlugin);
        }
        app.init_resource::<world::WorldLayout>()
            .init_resource::<world::ChunkIndex>()
            .init_resource::<edits::StoredChunks<T>>()
            .init_resource::<edits::PendingEdits<T>>()
            .add_observer(edits::set_voxel::<T>)
            .add_observer(reset_world::<T>)
            .add_observer(streaming::cleanup_removed_chunk::<T>);
    }
}

fn reset_world<T: VoxelCell>(
    _: bevy_ecs::prelude::On<session::SessionClosed>,
    mut index: bevy_ecs::prelude::ResMut<world::ChunkIndex>,
    mut stored: bevy_ecs::prelude::ResMut<edits::StoredChunks<T>>,
    mut edits: bevy_ecs::prelude::ResMut<edits::PendingEdits<T>>,
) {
    index.clear();
    stored.clear();
    edits.clear();
}
