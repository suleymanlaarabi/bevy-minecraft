//! Optional voxel presentation, independent of Minecraft and generation providers.
pub mod articulated;
pub mod assets;
pub mod build;
pub mod catalog;
pub mod diagnostics;
pub mod environment;
pub mod fluids;
pub mod fog;
pub mod material;
pub mod meshing;
pub mod mipmaps;
pub mod model_mesh;
pub mod models;
pub mod sort;
pub mod source;
pub mod streaming;
pub mod uploads;
pub mod visibility;
use bevy::prelude::*;
pub use source::RenderSource;
use std::marker::PhantomData;
pub const SHADOW_LAYER: usize = 1;
#[derive(
    Component, Clone, Copy, Debug, Default, bevy::render::extract_component::ExtractComponent,
)]
#[extract_app(bevy::render::RenderApp)]
pub struct VoxelViewer;
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PrepareVoxelAssets;
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoxelRenderSystems {
    Prepare,
    Build,
}
pub struct VoxelRenderPlugin<C: RenderSource>(PhantomData<C>);
impl<C: RenderSource> Default for VoxelRenderPlugin<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<C: RenderSource> Plugin for VoxelRenderPlugin<C> {
    fn build(&self, app: &mut App) {
        // Embedded WESL imports use package paths, which the asset loader does not
        // load recursively. Keep the shared modules loaded for every material.
        bevy::shader::load_shader_library!(app, "shaders/voxel_lighting.wesl");
        bevy::shader::load_shader_library!(app, "shaders/voxel_vertex.wesl");
        bevy::shader::load_shader_library!(app, "shaders/sky_stars.wesl");
        bevy::shader::load_shader_library!(app, "shaders/sky_moon.wesl");
        bevy::shader::load_shader_library!(app, "shaders/sky_clouds.wesl");
        bevy::shader::load_shader_library!(app, "shaders/sky_sun.wesl");
        bevy::shader::load_shader_library!(app, "shaders/model_shadow.wesl");
        bevy::shader::load_shader_library!(app, "shaders/model_vertex.wesl");
        bevy::shader::load_shader_library!(app, "shaders/voxel_prepass.wesl");
        bevy::shader::load_shader_library!(app, "shaders/horizon_fog.wesl");
        bevy::shader::load_shader_library!(app, "shaders/surface_animation.wesl");
        bevy::shader::load_shader_library!(app, "shaders/block_lighting.wesl");
        bevy::shader::load_shader_library!(app, "shaders/fog_lut.wesl");
        bevy::shader::load_shader_library!(app, "shaders/prepare_fog.wesl");
        bevy::shader::load_shader_library!(app, "shaders/voxel_fog.wesl");
        bevy::shader::load_shader_library!(app, "shaders/voxel_material.wesl");
        bevy::shader::load_shader_library!(app, "shaders/water_material.wesl");
        bevy::shader::load_shader_library!(app, "shaders/model_material.wesl");
        app.init_resource::<environment::CameraLiquid>()
            .init_resource::<environment::SkyLighting>()
            .init_resource::<environment::EnvironmentSettings>()
            .init_resource::<environment::ViewEffects>();
        app.init_resource::<streaming::ChunkArrivals>()
            .add_observer(reset)
            .add_observer(streaming::track_chunk_arrival)
            .add_observer(streaming::prepare_chunk)
            .add_observer(streaming::remove_chunk)
            .add_systems(
                PostUpdate,
                (
                    streaming::refresh_neighbors_of_spawned::<C>,
                    streaming::release_ready_chunks::<C>,
                )
                    .chain()
                    .in_set(VoxelRenderSystems::Prepare)
                    .in_set(hollow_engine::streaming::VoxelPipeline)
                    .after(hollow_engine::streaming::VoxelStreamSystems::Publish)
                    .before(hollow_engine::lighting::VoxelLighting),
            )
            .configure_sets(
                PostUpdate,
                VoxelRenderSystems::Build.after(hollow_engine::streaming::VoxelPipeline),
            );
        sort::register(app);
        visibility::register::<C>(app);
        uploads::register(app);
        fog::register(app);
        app.add_plugins((
            MaterialPlugin::<material::VoxelMaterial>::default(),
            MaterialPlugin::<material::VoxelCutoutMaterial>::default(),
            MaterialPlugin::<material::ModelMaterial>::default(),
            MaterialPlugin::<environment::HorizonFogMaterial>::default(),
        ))
        .add_systems(
            PostUpdate,
            assets::prepare_single_layer_arrays
                .after(bevy::asset::AssetEventSystems)
                .run_if(
                    on_message::<AssetEvent<Image>>
                        .or_else(on_message::<AssetEvent<material::ModelMaterial>>),
                ),
        )
        .add_systems(
            PostUpdate,
            (
                build::poll_builds::<C>,
                build::mark_changed_chunks::<C>,
                build::start_changed_builds::<C>,
            )
                .chain()
                .in_set(VoxelRenderSystems::Build)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(resource_exists::<C>),
        )
        .add_systems(
            Update,
            mipmaps::generate_block_mipmaps.run_if(
                on_message::<AssetEvent<Image>>
                    .or_else(on_message::<AssetEvent<material::VoxelMaterial>>)
                    .or_else(resource_exists::<mipmaps::MipmapJob>),
            ),
        );
    }
}

fn reset(
    _: On<hollow_engine::session::SessionClosed>,
    mut arrivals: ResMut<streaming::ChunkArrivals>,
    mut liquid: ResMut<environment::CameraLiquid>,
) {
    *arrivals = Default::default();
    *liquid = Default::default();
}
