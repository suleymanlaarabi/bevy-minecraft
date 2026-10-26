//! Voxel domain facade and the shared streaming/rebuild schedule.

mod api;
mod block_relations;
mod blocks;
mod chunk;
mod collision;
pub(crate) mod coordinates;
#[cfg(feature = "dev")]
mod diagnostics;
pub(crate) mod edits;
pub(crate) mod generation;
mod lighting;
mod loading;
mod persistence;
pub(crate) mod render;
pub(crate) mod rules;
mod settings;
pub(crate) mod state;
mod storage;
pub use state::VoxelState;
pub(crate) mod streaming;

use crate::{game::GameState, settings::GraphicsSettings};
pub use api::{VoxelCommandsExt, VoxelWorld};
use bevy::prelude::*;
pub use blocks::VoxelKind;
pub use chunk::VoxelViewer;
pub(crate) use chunk::{ChunkVoxels, VoxelChunk};
use collision::{poll_chunk_colliders, request_chunk_colliders};
pub use coordinates::chunk_at;
pub(crate) use coordinates::{CHUNK_SIZE, split_world_position};
pub use edits::SetVoxel;
use edits::set_voxel;
pub(crate) use loading::VoxelLoading;
pub use render::TexturePackId;
pub(crate) use render::assets::VoxelAssets;
use render::assets::apply_texture_pack;
pub(crate) use render::assets::prepare_assets;
use render::build::{mark_changed_chunks, poll_builds, start_changed_builds};
pub(crate) use render::{OPEN_AIR_SCATTERING, TexturePack, VoxelMaterial, terrain_radius};
use render::{follow_horizon_skirt, spawn_horizon_skirt, update_camera_environment};
pub use settings::VoxelSettings;
use streaming::{
    ChunkIndex, PendingChunks, PendingEdits, StoredChunks, StreamOffsets, StreamState,
    cleanup_removed_chunk, cleanup_voxel_world, poll_chunk_generations,
    refresh_neighbors_of_spawned, release_ready_chunks, stream_chunks,
};

/// Render layer of depth-only terrain: drawn into the sun's shadow maps, never by cameras.
pub(crate) const SHADOW_LAYER: usize = 1;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct VoxelPipeline;

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

        app.add_plugins(render::VoxelRenderPlugin)
            .insert_resource(self.settings.clone())
            .insert_resource(StreamOffsets::new(view_distance))
            .init_resource::<ChunkIndex>()
            .init_resource::<streaming::ChunkArrivals>()
            .init_resource::<collision::ColliderRequests>()
            .add_observer(collision::invalidate_collider)
            .add_observer(streaming::track_chunk_arrival)
            .init_resource::<StoredChunks>()
            .init_resource::<PendingChunks>()
            .init_resource::<PendingEdits>()
            .init_resource::<StreamState>()
            .init_resource::<lighting::LightingState>()
            .init_resource::<lighting::LightingVersion>()
            .init_resource::<lighting::BlockLightEdits>()
            .add_observer(lighting::edited_lighting)
            .add_observer(lighting::added_lighting)
            .add_observer(lighting::removed_lighting)
            .add_observer(apply_view_distance)
            .add_observer(apply_texture_pack)
            .add_observer(set_voxel)
            .add_observer(edits::edit_batch)
            .add_observer(edits::refresh_connection_border)
            .add_observer(cleanup_removed_chunk)
            .add_systems(PreStartup, prepare_assets)
            .add_systems(Update, stream_chunks.run_if(crate::world::streaming))
            .add_systems(
                Update,
                render::mipmaps::generate_block_mipmaps.run_if(
                    on_message::<AssetEvent<Image>>
                        .or_else(on_message::<AssetEvent<VoxelMaterial>>)
                        .or_else(resource_exists::<render::mipmaps::MipmapJob>),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    poll_chunk_generations,
                    refresh_neighbors_of_spawned,
                    release_ready_chunks,
                    lighting::update_block_light,
                    lighting::update_lighting,
                    // Release completed/stale task slots before scheduling their
                    // replacements, avoiding an idle frame at every completion.
                    poll_builds,
                    mark_changed_chunks,
                    start_changed_builds,
                    poll_chunk_colliders,
                    request_chunk_colliders,
                )
                    .chain()
                    .in_set(VoxelPipeline)
                    .after(crate::player::PlayerInteraction)
                    .before(bevy::transform::TransformSystems::Propagate)
                    .run_if(crate::world::streaming),
            )
            .add_systems(
                PostUpdate,
                update_camera_environment.run_if(crate::world::streaming),
            )
            .add_systems(OnEnter(GameState::Game), spawn_horizon_skirt)
            .add_systems(
                PostUpdate,
                follow_horizon_skirt
                    .after(update_camera_environment)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(OnExit(GameState::Game), cleanup_voxel_world);

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

#[cfg(test)]
mod tests;
