//! Voxel domain facade and the shared streaming/rebuild schedule.

mod api;
mod chunk;
#[cfg(feature = "dev")]
mod diagnostics;
pub(crate) mod edits;
pub(crate) mod generation;
mod loading;
pub(crate) mod render;
pub(crate) mod rules;
mod settings;
pub use hollow_worldgen::VoxelState;
pub(crate) mod streaming;

use crate::{game::GameState, settings::GraphicsSettings};
pub use api::{VoxelCommandsExt, VoxelWorld};
use bevy::prelude::*;
pub use chunk::VoxelViewer;
pub(crate) use chunk::{ChunkVoxels, VoxelChunk};
pub use edits::SetVoxel;
pub use hollow_core::coordinates::chunk_at;
pub(crate) use hollow_core::coordinates::{CHUNK_SIZE, split_world_position};
pub(crate) use hollow_engine::streaming::VoxelPipeline;
use hollow_render::VoxelRenderSystems;
pub(crate) use hollow_render::assets::VoxelAssets;
use hollow_render::environment::{
    follow_horizon_skirt, spawn_horizon_skirt, update_camera_environment,
};
pub(crate) use hollow_render::{environment::OPEN_AIR_SCATTERING, material::VoxelMaterial};
pub use hollow_worldgen::VoxelKind;
pub(crate) use loading::VoxelLoading;
use render::MinecraftRenderSource;
pub use render::TexturePackId;
use render::assets::apply_texture_pack;
pub(crate) use render::assets::prepare_assets;
pub(crate) use render::{TexturePack, terrain_radius};
pub use settings::VoxelSettings;
use streaming::{StreamOffsets, StreamState, cleanup_voxel_world};

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
            hollow_engine::VoxelWorldPlugin::<VoxelState>::default(),
            hollow_worldgen::WorldGenerationPlugin,
            hollow_engine::streaming::StreamingPlugin::<generation::WorldGenerator>::default(),
            hollow_engine::lighting::VoxelLightingPlugin::<VoxelState>::default(),
            hollow_render::VoxelRenderPlugin::<MinecraftRenderSource>::default(),
        ))
        .insert_resource(self.settings.clone())
        .insert_resource(hollow_engine::streaming::StreamSettings {
            spawn_budget_per_frame: self.settings.spawn_budget_per_frame,
            despawn_budget_per_frame: self.settings.despawn_budget_per_frame,
        })
        .insert_resource(StreamOffsets::new(view_distance))
        .init_resource::<edits::ConnectionBorders>()
        .add_observer(apply_view_distance)
        .add_observer(apply_texture_pack)
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
                .after(hollow_engine::blocks::FinalizeBlockContent)
                .after(hollow_render::PrepareVoxelAssets),
        )
        .add_systems(
            Update,
            (streaming::update_demand, streaming::publish_available)
                .chain()
                .before(hollow_engine::streaming::VoxelStreamSystems::Demand)
                .run_if(crate::world::streaming),
        )
        .configure_sets(
            Update,
            hollow_engine::streaming::VoxelStreamSystems::Demand.run_if(crate::world::streaming),
        )
        .configure_sets(
            PostUpdate,
            VoxelPipeline
                .after(crate::player::PlayerInteraction)
                .before(hollow_engine::tasks::VoxelTasks::Requests)
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
            (hollow_engine::session::open_session, spawn_horizon_skirt).chain(),
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
                cleanup_voxel_world,
                streaming::reset_voxel_world,
                hollow_engine::session::close_session,
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

fn validate_layout(layout: Res<hollow_engine::world::WorldLayout>) {
    assert_eq!(
        layout.0,
        hollow_core::coordinates::ColumnLayout::default(),
        "Minecraft generation requires its native 384-block column layout"
    );
}
