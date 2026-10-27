//! Workbench sessions own no items; ingredients remain on the player through unloads.
use super::{
    containers::{BlockEntity, BlockState, Dormant, ViewingContainer},
    interaction::BlockUse,
};
use crate::{
    inventory::{CraftingGrid, InventoryState},
    voxel::{VoxelKind, VoxelWorld},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_features::motion::FixedPosition;

#[derive(Component, Default)]
pub(crate) struct CraftingTable;
#[derive(Component)]
#[relationship(relationship_target = WorkbenchViewers)]
pub(crate) struct ViewingWorkbench(pub Entity);
#[derive(Component, Default)]
#[relationship_target(relationship = ViewingWorkbench)]
pub(crate) struct WorkbenchViewers(Vec<Entity>);

#[derive(SystemParam)]
pub(crate) struct WorkbenchAccess<'w, 's> {
    blocks: Query<
        'w,
        's,
        (&'static BlockEntity, &'static BlockState),
        (With<CraftingTable>, Without<Dormant>),
    >,
    world: VoxelWorld<'w, 's>,
}
impl WorkbenchAccess<'_, '_> {
    pub fn valid(&self, entity: Entity, position: Vec3) -> bool {
        self.blocks.get(entity).is_ok_and(|(block, state)| {
            state.0.kind() == VoxelKind::from_native(pumpkin_data::BlockId::CRAFTING_TABLE)
                && self.world.state(block.position) == Some(state.0)
                && position.distance_squared(block.position.as_vec3() + Vec3::splat(0.5)) <= 25.0
        })
    }
}
pub(crate) fn open(
    event: On<BlockUse>,
    mut commands: Commands,
    access: WorkbenchAccess,
    mut actors: Query<(&FixedPosition, &mut CraftingGrid), Without<ViewingContainer>>,
    mut next: ResMut<NextState<InventoryState>>,
) {
    let Ok((motion, mut grid)) = actors.get_mut(event.actor) else {
        return;
    };
    if event.state.kind() != VoxelKind::from_native(pumpkin_data::BlockId::CRAFTING_TABLE)
        || !access.valid(event.entity, motion.current.as_vec3())
    {
        return;
    }
    grid.side = 3;
    commands
        .entity(event.actor)
        .insert(ViewingWorkbench(event.entity));
    next.set(InventoryState::Open);
}
pub(crate) fn validate(
    mut commands: Commands,
    sessions: Query<(Entity, &ViewingWorkbench, &FixedPosition)>,
    access: WorkbenchAccess,
) {
    for (entity, session, motion) in &sessions {
        if !access.valid(session.0, motion.current.as_vec3()) {
            commands.entity(entity).remove::<ViewingWorkbench>();
        }
    }
}

pub struct WorkbenchPlugin;
impl Plugin for WorkbenchPlugin {
    fn build(&self, app: &mut App) {
        let kind = VoxelKind::from_native(pumpkin_data::BlockId::CRAFTING_TABLE);
        app.world_mut()
            .resource_mut::<hollow_engine::blocks::BlockRegistrations>()
            .register_bundle::<CraftingTable>(hollow_core::BlockId(kind.index() as u32), true);
        app.add_observer(open)
            .add_observer(super::containers::session_closed::<ViewingWorkbench>)
            .add_systems(
                PostUpdate,
                validate
                    .in_set(super::GameplaySystems::Validation)
                    .run_if(crate::world::playing),
            )
            .add_systems(
                OnExit(InventoryState::Open),
                super::containers::close_sessions::<ViewingWorkbench>,
            );
    }
}
