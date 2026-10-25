//! Voxel rendering implementation. VoxelPlugin owns the shared streaming schedule.

pub(super) mod assets;
pub(super) mod build;
mod environment;
mod material;
mod meshing;
pub(super) mod mipmaps;
pub(crate) mod model_mesh;
pub(crate) mod models;
mod sort;
mod texture_pack;

use bevy::prelude::*;
pub(super) use environment::update_underwater_effect;
pub(crate) use material::{ModelMaterial, VoxelMaterial};
pub(crate) use texture_pack::TexturePack;
pub use texture_pack::TexturePackId;

pub(super) struct VoxelRenderPlugin;

impl Plugin for VoxelRenderPlugin {
    fn build(&self, app: &mut App) {
        sort::register(app);
        app.add_plugins((
            MaterialPlugin::<VoxelMaterial>::default(),
            MaterialPlugin::<material::VoxelCutoutMaterial>::default(),
            MaterialPlugin::<ModelMaterial>::default(),
        ));
    }
}
