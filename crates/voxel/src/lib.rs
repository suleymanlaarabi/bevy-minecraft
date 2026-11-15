//! Public, content-independent composition for voxel applications.
pub use hollow_core as core;
pub use hollow_core::{BlockId, FluidId, ResourceId, ShapeId, StateId, VoxelCell};
pub use hollow_engine as engine;
pub use hollow_engine::*;
pub use hollow_features as features;
#[cfg(feature = "render")]
pub use hollow_render as render;
pub mod character;
pub mod navigation;
pub mod spatial;

use bevy::prelude::*;
use std::marker::PhantomData;

/// World access, streaming and lighting with a statically dispatched terrain provider.
pub struct VoxelPlugin<P: streaming::TerrainProvider>(PhantomData<P>);
impl<P: streaming::TerrainProvider> Default for VoxelPlugin<P> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<P: streaming::TerrainProvider> Plugin for VoxelPlugin<P> {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            VoxelWorldPlugin::<P::Cell>::default(),
            streaming::StreamingPlugin::<P>::default(),
            lighting::VoxelLightingPlugin::<P::Cell>::default(),
        ));
    }
}
