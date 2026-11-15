//! Voxel domain facade and the shared streaming/rebuild schedule.

mod api;
#[cfg(feature = "dev")]
mod diagnostics;
pub(crate) mod edits;
pub(crate) mod generation;
pub(crate) mod render;
pub(crate) mod rules;
mod settings;
pub use hollow_worldgen::VoxelState;
pub(crate) mod streaming;

use crate::{game::GameState, settings::GraphicsSettings};
pub use api::{VoxelCommandsExt, VoxelWorld};
use bevy::prelude::*;
pub use bevy_voxel::render::VoxelViewer;
pub(crate) type ChunkVoxels = bevy_voxel::chunk::ChunkVoxels<VoxelState>;
pub(crate) use bevy_voxel::chunk::VoxelChunk;
pub use bevy_voxel::core::coordinates::chunk_at;
pub(crate) use bevy_voxel::core::coordinates::{CHUNK_SIZE, split_world_position};
pub(crate) use bevy_voxel::engine::streaming::VoxelPipeline;
use bevy_voxel::render::VoxelRenderSystems;
pub(crate) use bevy_voxel::render::assets::VoxelAssets;
use bevy_voxel::render::environment::{
    follow_horizon_skirt, spawn_horizon_skirt, update_camera_environment,
};
pub(crate) use bevy_voxel::render::{environment::OPEN_AIR_SCATTERING, material::VoxelMaterial};
pub use edits::SetVoxel;
pub use hollow_worldgen::VoxelKind;
pub(crate) type VoxelLoading<'w, 's> = bevy_voxel::render::readiness::WorldReadiness<
    'w,
    's,
    VoxelState,
    crate::creatures::ColumnActors,
>;
use render::MinecraftRenderSource;
pub(crate) use render::assets::prepare_assets;
pub(crate) use render::{MinecraftTextures, terrain_radius};
pub use settings::VoxelSettings;
use streaming::{StreamOffsets, StreamState};

pub struct VoxelPlugin {
    settings: VoxelSettings,
}

impl VoxelPlugin {
    pub const fn new(settings: VoxelSettings) -> Self {
        Self { settings }
    }
}

impl Default for VoxelPlugin {
    fn default() -> Self {
        Self::new(VoxelSettings::default())
    }
}

impl Plugin for VoxelPlugin {
    fn build(&self, app: &mut App) {
        let view_distance = app.world().get_resource::<GraphicsSettings>().map_or_else(
            || GraphicsSettings::default().effective_view_distance(),
            GraphicsSettings::effective_view_distance,
        );

        app.add_plugins((
            bevy_voxel::VoxelPlugin::<generation::WorldGenerator>::default(),
            hollow_worldgen::WorldGenerationPlugin,
            bevy_voxel::render::VoxelRenderPlugin::<MinecraftRenderSource>::default(),
        ))
        .insert_resource(self.settings.clone())
        .insert_resource(bevy_voxel::engine::streaming::StreamSettings {
            spawn_budget_per_frame: self.settings.spawn_budget_per_frame,
            despawn_budget_per_frame: self.settings.despawn_budget_per_frame,
        })
        .insert_resource(StreamOffsets::new(view_distance))
        .init_resource::<edits::ConnectionBorders>()
        .add_observer(apply_view_distance)
        .add_observer(edits::enqueue_neighbors)
        .add_observer(edits::edit_batch)
        .add_observer(edits::queue_connection_border)
        .add_systems(
            PostUpdate,
            edits::refresh_connection_border
                .after(VoxelPipeline)
                .before(crate::block_behavior::GameplaySystems::Topology)
                .run_if(crate::world::streaming),
        )
        .add_systems(
            PreStartup,
            (validate_layout, render::initialize_source, prepare_assets)
                .chain()
                .after(bevy_voxel::engine::blocks::FinalizeBlockContent)
                .after(bevy_voxel::render::PrepareVoxelAssets),
        )
        .add_systems(
            Update,
            (streaming::update_demand, streaming::publish_available)
                .chain()
                .before(bevy_voxel::engine::streaming::VoxelStreamSystems::Demand)
                .run_if(crate::world::streaming),
        )
        .configure_sets(
            Update,
            bevy_voxel::engine::streaming::VoxelStreamSystems::Demand
                .run_if(crate::world::streaming),
        )
        .configure_sets(
            PostUpdate,
            VoxelPipeline
                .after(crate::player::PlayerInteraction)
                .before(bevy_voxel::engine::tasks::VoxelTasks::Requests)
                .run_if(crate::world::streaming),
        )
        .configure_sets(
            PostUpdate,
            VoxelRenderSystems::Build.run_if(crate::world::streaming),
        )
        .add_systems(
            PostUpdate,
            (
                render::sync_environment,
                update_camera_environment::<MinecraftRenderSource>,
            )
                .chain()
                .run_if(crate::world::streaming),
        )
        .add_systems(
            OnEnter(GameState::Game),
            (
                bevy_voxel::engine::session::open_session,
                spawn_horizon_skirt,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            follow_horizon_skirt
                .after(update_camera_environment::<MinecraftRenderSource>)
                .before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            OnExit(GameState::Game),
            (
                bevy_voxel::streaming::cleanup_world::<generation::WorldGenerator>,
                streaming::clear_generation_view,
                streaming::reset_voxel_world,
                bevy_voxel::engine::session::close_session,
            )
                .chain(),
        );

        #[cfg(feature = "dev")]
        diagnostics::register(app);
    }
}

fn apply_view_distance(
    _insert: On<Insert<GraphicsSettings>>,
    settings: Res<GraphicsSettings>,
    mut offsets: ResMut<StreamOffsets>,
    mut state: ResMut<StreamState>,
) {
    let view_distance = settings.effective_view_distance();
    if offsets.render_radius == view_distance {
        return;
    }
    *offsets = StreamOffsets::new(view_distance);
    state.center = None;
}

fn validate_layout(layout: Res<bevy_voxel::engine::world::WorldLayout>) {
    assert_eq!(
        layout.0,
        bevy_voxel::core::coordinates::ColumnLayout::default(),
        "Minecraft generation requires its native 384-block column layout"
    );
}
