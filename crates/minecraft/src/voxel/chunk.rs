use bevy::prelude::*;
pub type ChunkVoxels = hollow_engine::chunk::ChunkVoxels<super::VoxelState>;
pub use hollow_engine::chunk::VoxelChunk;

#[derive(
    Component, Clone, Copy, Debug, Default, bevy::render::extract_component::ExtractComponent,
)]
#[extract_app(bevy::render::RenderApp)]
pub struct VoxelViewer;
