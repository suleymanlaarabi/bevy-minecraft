//! Workbench sessions own no items; ingredients remain on the player through unloads.
use super::{
    containers::{BlockEntity, BlockState, Dormant, ViewingContainer},
    interaction::BlockUse,
};
use crate::{
    character::CharacterMotion,
    inventory::{CraftingGrid, InventoryState},
    voxel::{VoxelKind, VoxelWorld},
};
use bevy::{ecs::system::SystemParam, prelude::*};

#[derive(Component)]
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
            state.0.kind().0 == pumpkin_data::BlockId::CRAFTING_TABLE
                && self.world.state(block.position) == Some(state.0)
                && position.distance_squared(block.position.as_vec3() + Vec3::splat(0.5)) <= 25.0
        })
    }
}
pub(crate) fn open(
    event: On<BlockUse>,
    mut commands: Commands,
    access: WorkbenchAccess,
    mut actors: Query<(&CharacterMotion, &mut CraftingGrid), Without<ViewingContainer>>,
    mut next: ResMut<NextState<InventoryState>>,
) {
    let Ok((motion, mut grid)) = actors.get_mut(event.actor) else {
        return;
    };
    if event.state.kind() != VoxelKind(pumpkin_data::BlockId::CRAFTING_TABLE)
        || !access.valid(event.entity, motion.position.as_vec3())
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
    sessions: Query<(Entity, &ViewingWorkbench, &CharacterMotion)>,
    access: WorkbenchAccess,
) {
    for (entity, session, motion) in &sessions {
        if !access.valid(session.0, motion.position.as_vec3()) {
            commands.entity(entity).remove::<ViewingWorkbench>();
        }
    }
}
pub(crate) fn closed(
    _: On<Remove<ViewingWorkbench>>,
    next: Option<ResMut<NextState<InventoryState>>>,
    mut actions: ResMut<crate::inventory::access::InventoryActions>,
) {
    if let Some(mut next) = next {
        next.set(InventoryState::Closed);
    }
    actions.clear();
}
pub(crate) fn close(mut commands: Commands, sessions: Query<Entity, With<ViewingWorkbench>>) {
    for entity in &sessions {
        commands.entity(entity).remove::<ViewingWorkbench>();
    }
}
