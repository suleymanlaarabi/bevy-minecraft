//! Voxel rendering implementation. VoxelPlugin owns the shared streaming schedule.

pub(super) mod assets;
pub(super) mod build;
mod environment;
mod fluids;
mod material;
mod meshing;
pub(super) mod mipmaps;
pub(crate) mod model_mesh;
pub(crate) mod models;
mod sort;
mod texture_pack;

use bevy::prelude::*;
pub(crate) use environment::{OPEN_AIR_SCATTERING, terrain_radius};
pub(super) use environment::{
    follow_horizon_skirt, spawn_horizon_skirt, update_camera_environment,
};
pub(crate) use material::{ModelMaterial, VoxelMaterial};
pub(crate) use texture_pack::TexturePack;
pub use texture_pack::TexturePackId;

pub(super) struct VoxelRenderPlugin;

impl Plugin for VoxelRenderPlugin {
    fn build(&self, app: &mut App) {
        sort::register(app);
        app.add_systems(
            PostUpdate,
            assets::prepare_single_layer_arrays
                .after(bevy::asset::AssetEventSystems)
                .run_if(
                    on_message::<AssetEvent<Image>>
                        .or_else(on_message::<AssetEvent<ModelMaterial>>),
                ),
        );
        app.init_resource::<environment::CameraLiquid>()
            .add_plugins((
                MaterialPlugin::<VoxelMaterial>::default(),
                MaterialPlugin::<material::VoxelCutoutMaterial>::default(),
                MaterialPlugin::<ModelMaterial>::default(),
                MaterialPlugin::<environment::HorizonFogMaterial>::default(),
            ));
    }
}
