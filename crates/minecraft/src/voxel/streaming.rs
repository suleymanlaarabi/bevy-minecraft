use super::{ChunkVoxels, VoxelChunk, VoxelViewer, generation::WorldGenerator};
use bevy::prelude::*;
use hollow_core::coordinates::chunk_at;
use hollow_engine::edits::LitEdits;
pub use hollow_engine::streaming::PendingChunks;
pub use hollow_engine::streaming::{AwaitingNeighbors, StreamOffsets, StreamState};
use hollow_render::build::ChunkSections;

/// Gameplay records being loaded and materialized around shared generated sections.
pub use hollow_engine::world::ChunkIndex;
pub(crate) type StoredChunks = hollow_engine::edits::StoredChunks<super::VoxelState>;
/// Edits aimed at chunks that are neither loaded nor stored, replayed when they spawn.
/// Generating a chunk just to edit it would block the main thread.
pub(crate) type PendingEdits = hollow_engine::edits::PendingEdits<super::VoxelState>;

pub(super) fn cleanup_voxel_world(
    mut commands: Commands,
    generator: Option<Res<WorldGenerator>>,
    chunks: Query<Entity, With<VoxelChunk>>,
    generations: Query<Entity, With<hollow_engine::streaming::ChunkGeneration<WorldGenerator>>>,
) {
    for entity in &chunks {
        unload_chunk(&mut commands, entity);
    }
    for entity in &generations {
        commands.entity(entity).despawn();
    }
    // Release view demand; active generation retains its reservations until return.
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
            &hollow_features::motion::FixedPosition,
            &crate::survival::Health,
        ),
        With<crate::player::Player>,
    >,
    simulation: Res<crate::settings::SimulationSettings>,
    mut demand: ResMut<hollow_engine::streaming::StreamDemand>,
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
    session: Option<Res<hollow_engine::session::ActiveSession>>,
    mut outgoing: MessageWriter<hollow_engine::streaming::TerrainAvailable>,
) {
    let (Some(generator), Some(session)) = (generator, session) else {
        return;
    };
    for event in incoming
        .read()
        .filter(|event| event.session == generator.runtime.id())
    {
        outgoing.write(hollow_engine::streaming::TerrainAvailable {
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
impl hollow_engine::streaming::TerrainProvider for WorldGenerator {
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
        budget: hollow_engine::tasks::VoxelTaskBudget,
    ) -> impl std::future::Future<
        Output = hollow_engine::streaming::ChunkLoad<Self::Cell, Self::Attachment>,
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
