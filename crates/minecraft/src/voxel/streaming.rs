use super::{ChunkVoxels, VoxelChunk, VoxelViewer, generation::WorldGenerator};
use bevy::prelude::*;
use bevy_voxel::core::coordinates::chunk_at;
use bevy_voxel::engine::edits::LitEdits;
pub use bevy_voxel::engine::streaming::PendingChunks;
pub use bevy_voxel::engine::streaming::{StreamOffsets, StreamState};
use bevy_voxel::render::build::ChunkSections;

/// Gameplay records being loaded and materialized around shared generated sections.
pub use bevy_voxel::engine::world::ChunkIndex;
pub(crate) type StoredChunks = bevy_voxel::engine::edits::StoredChunks<super::VoxelState>;
/// Edits aimed at chunks that are neither loaded nor stored, replayed when they spawn.
/// Generating a chunk just to edit it would block the main thread.
pub(crate) type PendingEdits = bevy_voxel::engine::edits::PendingEdits<super::VoxelState>;

pub(super) fn clear_generation_view(generator: Option<Res<WorldGenerator>>) {
    if let Some(generator) = generator {
        generator.clear_view();
    }
}

/// Run after unload commands and their observers have finished writing back.
pub(crate) fn reset_voxel_world(mut borders: ResMut<super::edits::ConnectionBorders>) {
    borders.clear();
}

/// Release presentation while the generation graph retains the procedural column.
fn unload_chunk(commands: &mut Commands, entity: Entity) {
    let mut entity = commands.entity(entity);
    // Observers capture edits before identity and ownership relationships disappear.
    entity.remove::<ChunkVoxels>();
    entity.remove::<VoxelChunk>();
    entity.remove::<(
        hollow_worldgen::runtime::ChunkRetention,
        crate::block_behavior::containers::LoadedColumn,
        LitEdits,
        super::edits::EditedChunk,
    )>();
}

/// The application chooses its focus; the streamer knows neither health nor player types.
pub fn update_demand(
    viewer: Single<&GlobalTransform, With<VoxelViewer>>,
    player: Query<
        (
            &bevy_voxel::features::motion::FixedPosition,
            &crate::survival::Health,
        ),
        With<crate::player::Player>,
    >,
    simulation: Res<crate::settings::SimulationSettings>,
    mut demand: ResMut<bevy_voxel::engine::streaming::StreamDemand>,
) {
    let center = chunk_at(viewer.translation());
    demand.center = Some(center);
    demand.simulation_center = Some(
        player
            .single()
            .ok()
            .filter(|(_, health)| health.alive())
            .map_or(center, |(motion, _)| chunk_at(motion.current.as_vec3())),
    );
    demand.simulation_radius = simulation.effective_distance();
}
pub fn publish_available(
    mut incoming: MessageReader<hollow_worldgen::runtime::ChunkAvailable>,
    generator: Option<Res<WorldGenerator>>,
    session: Option<Res<bevy_voxel::engine::session::ActiveSession>>,
    mut outgoing: MessageWriter<bevy_voxel::engine::streaming::TerrainAvailable>,
) {
    let (Some(generator), Some(session)) = (generator, session) else {
        return;
    };
    for event in incoming
        .read()
        .filter(|event| event.session == generator.runtime.id())
    {
        outgoing.write(bevy_voxel::engine::streaming::TerrainAvailable {
            session: session.0,
            position: event.position,
        });
    }
}
#[derive(bevy::ecs::system::SystemParam)]
pub struct RetirementPolicy<'w, 's> {
    pressure: Res<'w, crate::simulation::SimulationDiagnostics>,
    terrain: Query<
        'w,
        's,
        (
            &'static ChunkVoxels,
            &'static crate::block_behavior::containers::LoadedColumn,
        ),
    >,
    gameplay: Query<
        'w,
        's,
        (
            &'static crate::block_behavior::containers::GameplayColumn,
            &'static crate::simulation::ColumnSimulation,
            Option<&'static crate::block_behavior::containers::BlockEntities>,
            Option<&'static crate::simulation::SimulatedEntities>,
            Option<&'static crate::creatures::spawning::ColumnBiomes>,
        ),
    >,
    sections: Query<'w, 's, &'static ChunkSections>,
    creatures: crate::creatures::CreatureSnapshots<'w, 's>,
}
impl bevy_voxel::engine::streaming::TerrainProvider for WorldGenerator {
    type Cell = super::VoxelState;
    type Attachment = crate::block_behavior::containers::PreparedColumn;
    type Policy = RetirementPolicy<'static, 'static>;
    fn set_view(&self, center: IVec2, positions: Vec<IVec2>) {
        self.runtime.set_data_view(center, positions);
    }
    fn is_ready(&self, position: IVec2) -> bool {
        self.is_ready(position)
    }
    fn storage_revision(&self, position: IVec2) -> Option<u64> {
        self.storage_revision(position)
    }
    fn entity(&self, _commands: &mut Commands, position: IVec2) -> Option<Entity> {
        self.runtime.entity(position)
    }
    fn load(
        &self,
        position: IVec2,
        budget: bevy_voxel::engine::tasks::VoxelTaskBudget,
    ) -> impl std::future::Future<
        Output = bevy_voxel::engine::streaming::ChunkLoad<Self::Cell, Self::Attachment>,
    > + Send
    + 'static {
        let generator = self.clone();
        async move { generator.loaded_gameplay_chunk(position, budget).await }
    }
    fn materialize(
        &self,
        commands: &mut Commands,
        entity: Entity,
        position: IVec2,
        attachment: Self::Attachment,
    ) {
        // Retention is inserted together with the column link: one archetype move.
        crate::block_behavior::containers::spawn_for_chunk(commands, entity, position, attachment);
    }
    fn release(&self, commands: &mut Commands, entity: Entity) {
        unload_chunk(commands, entity);
    }
    fn retirement_budget(
        policy: &bevy::ecs::system::SystemParamItem<Self::Policy>,
    ) -> (usize, usize) {
        (
            policy.pressure.pending_write_bytes,
            crate::simulation::MAX_RETIRED_BYTES,
        )
    }
    fn retained_bytes(
        policy: &bevy::ecs::system::SystemParamItem<Self::Policy>,
        entity: Entity,
    ) -> usize {
        policy.terrain.get(entity).map_or(0, |(voxels, owner)| {
            let terrain = if voxels.save_revision != voxels.saved_revision {
                voxels.retained_bytes()
            } else {
                0
            };
            terrain
                + policy.gameplay.get(owner.0).map_or(
                    0,
                    |(column, work, blocks, mobiles, biomes)| {
                        let mobile_bytes = policy.creatures.retirement_bytes(mobiles);
                        let retained = if column.revision != column.saved_revision {
                            work.estimated_bytes()
                                + blocks.map_or(0, |blocks| blocks.len())
                                    * crate::block_behavior::containers::RETAINED_BLOCK_BYTES
                                + column.application.retained_bytes()
                                + biomes.map_or(0, |b| b.0.len())
                        } else {
                            0
                        };
                        retained + mobile_bytes
                    },
                )
        })
    }
    fn retirement_weight(
        policy: &bevy::ecs::system::SystemParamItem<Self::Policy>,
        entity: Entity,
    ) -> usize {
        policy
            .sections
            .get(entity)
            .map_or(1, ChunkSections::mesh_count)
    }
}
