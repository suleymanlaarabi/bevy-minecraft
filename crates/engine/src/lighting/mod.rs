//! Content-independent snapshot lighting over shared voxel sections.
mod propagation;
mod snapshots;
pub use snapshots::{LightingChunk, LightingSnapshots, LightingUpdate};

pub use propagation::LightingMode;

pub mod runtime;
use bevy_app::{App, Plugin, PostUpdate};
use bevy_ecs::prelude::*;
use hollow_core::VoxelCell;
use std::marker::PhantomData;
#[derive(Event)]
pub struct ChunkLightChanged(pub Entity);
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoxelLighting;
pub struct VoxelLightingPlugin<T: VoxelCell>(PhantomData<T>);
impl<T: VoxelCell> Default for VoxelLightingPlugin<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<T: VoxelCell> Plugin for VoxelLightingPlugin<T> {
    fn build(&self, app: &mut App) {
        app.init_resource::<runtime::LightingState<T>>()
            .add_observer(reset::<T>)
            .init_resource::<runtime::LightingVersion>()
            .init_resource::<runtime::BlockLightEdits>()
            .add_observer(runtime::edited_lighting::<T>)
            .add_observer(runtime::added_lighting)
            .add_observer(runtime::removed_lighting)
            .add_systems(
                PostUpdate,
                (
                    runtime::update_block_light::<T>,
                    runtime::update_lighting::<T>,
                )
                    .chain()
                    .in_set(VoxelLighting)
                    .in_set(crate::streaming::VoxelPipeline)
                    .after(crate::streaming::VoxelStreamSystems::Publish),
            );
    }
}

fn reset<T: VoxelCell>(
    _: On<crate::session::SessionClosed>,
    mut state: ResMut<runtime::LightingState<T>>,
    mut edits: ResMut<runtime::BlockLightEdits>,
    mut version: ResMut<runtime::LightingVersion>,
) {
    let retired = std::mem::take(&mut *state);
    bevy_tasks::IoTaskPool::get()
        .spawn(async move {
            drop(retired);
        })
        .detach();
    *edits = Default::default();
    *version = Default::default();
}
