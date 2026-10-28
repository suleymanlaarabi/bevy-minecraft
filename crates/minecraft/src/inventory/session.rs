//! One station relationship and access boundary for every inventory screen.
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_features::motion::FixedPosition;

use super::{CraftingGrid, InventoryState, access::InventoryAction};
use crate::block_behavior::{
    containers::{BlockEntity, BlockState, ContainerTopology, Dormant},
    interaction::BlockUse,
};

/// Screen capabilities live on the station, never on each viewer or UI slot.
#[derive(Component, Clone, Copy)]
pub enum InventoryStation {
    Crafting { side: u8 },
    Container,
}

impl Default for InventoryStation {
    fn default() -> Self {
        Self::Crafting { side: 3 }
    }
}

#[derive(Component)]
#[relationship(relationship_target = StationViewers)]
pub struct InventorySession(pub Entity);

#[derive(Component, Default, Deref)]
#[relationship_target(relationship = InventorySession)]
pub struct StationViewers(Vec<Entity>);

/// Resolved on the stack at access boundaries; no mirrored inventory or topology.
pub(crate) struct InventoryLayout {
    pub crafting_side: u8,
    pub containers: [Option<Entity>; 2],
}

impl Default for InventoryLayout {
    fn default() -> Self {
        Self {
            crafting_side: 2,
            containers: [None; 2],
        }
    }
}

#[derive(SystemParam)]
pub(crate) struct StationAccess<'w, 's> {
    topology: ContainerTopology<'w, 's>,
    world: crate::voxel::VoxelWorld<'w, 's>,
    stations: Query<
        'w,
        's,
        (
            &'static InventoryStation,
            &'static BlockEntity,
            &'static BlockState,
        ),
        Without<Dormant>,
    >,
}

impl StationAccess<'_, '_> {
    pub fn layout(
        &self,
        session: Option<&InventorySession>,
        actor: Vec3,
    ) -> Option<InventoryLayout> {
        let Some(session) = session else {
            return Some(InventoryLayout::default());
        };
        let (station, block, state) = self.stations.get(session.0).ok()?;
        match *station {
            InventoryStation::Crafting { side } => ((1..=3).contains(&side)
                && block.accessible(&self.world, state.0, actor))
            .then_some(InventoryLayout {
                crafting_side: side,
                containers: [None; 2],
            }),
            InventoryStation::Container => Some(InventoryLayout {
                containers: self.topology.owners(session.0, actor)?,
                ..default()
            }),
        }
    }
}

pub(super) struct InventorySessionPlugin;

impl Plugin for InventorySessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(open)
            .add_observer(session_closed)
            .add_systems(OnExit(InventoryState::Open), close_sessions)
            .add_systems(
                PostUpdate,
                validate
                    .in_set(crate::block_behavior::GameplaySystems::Validation)
                    .run_if(crate::world::playing),
            );
    }
}

fn open(
    event: On<BlockUse>,
    mut commands: Commands,
    access: StationAccess,
    mut actors: Query<(&FixedPosition, &mut CraftingGrid), Without<InventorySession>>,
    mut next: ResMut<NextState<InventoryState>>,
) {
    let Ok((station, _, state)) = access.stations.get(event.entity) else {
        return;
    };
    if state.0 != event.state {
        return;
    }
    let owner = match station {
        InventoryStation::Container => access.topology.canonical(event.entity),
        InventoryStation::Crafting { .. } => Some(event.entity),
    };
    let Some(owner) = owner else {
        return;
    };
    let Ok((position, grid)) = actors.get_mut(event.actor) else {
        return;
    };
    let Some(layout) = access.layout(Some(&InventorySession(owner)), position.current.as_vec3())
    else {
        return;
    };
    grid.map_unchanged(|grid| &mut grid.side)
        .set_if_neq(usize::from(layout.crafting_side));
    commands.entity(event.actor).insert(InventorySession(owner));
    next.set(InventoryState::Open);
}

fn validate(
    mut commands: Commands,
    sessions: Query<(Entity, &InventorySession, &FixedPosition, &CraftingGrid)>,
    access: StationAccess,
) {
    for (entity, session, position, grid) in &sessions {
        if access
            .layout(Some(session), position.current.as_vec3())
            .is_none_or(|layout| usize::from(layout.crafting_side) != grid.side)
        {
            commands.entity(entity).remove::<InventorySession>();
        }
    }
}

fn session_closed(
    _: On<Remove<InventorySession>>,
    next: Option<ResMut<NextState<InventoryState>>>,
    mut actions: ResMut<Messages<InventoryAction>>,
) {
    if let Some(mut next) = next {
        next.set(InventoryState::Closed);
    }
    actions.clear();
}

fn close_sessions(mut commands: Commands, sessions: Query<Entity, With<InventorySession>>) {
    for entity in &sessions {
        commands.entity(entity).remove::<InventorySession>();
    }
}
