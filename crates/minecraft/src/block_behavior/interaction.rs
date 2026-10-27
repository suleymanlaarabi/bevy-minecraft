//! Targeted observers handle capabilities; voxel commits own consumption and drops.
use super::{
    catalog::{SIDE_FACING, SIDES, state_behavior},
    containers::{
        BlockEntity, BlockLookup, BlockState, Chest, ContainerTopology, Door, Dormant, Gate,
        ManualOpen, ViewingContainer,
    },
};
use crate::{
    character::{Aabb, CharacterBody, CharacterMotion, CharacterState, GameCharacter},
    voxel::{
        VoxelState, VoxelWorld,
        edits::{CheckedEdit, EditBatch},
    },
};
use bevy::{ecs::system::SystemParam, prelude::*};
use pumpkin_data::Property as P;

#[derive(EntityEvent)]
pub struct BlockUse {
    pub entity: Entity,
    pub actor: Entity,
    pub state: VoxelState,
    pub look: Vec3,
}
#[derive(SystemParam)]
pub(crate) struct BlockTargets<'w, 's> {
    lookup: BlockLookup<'w, 's>,
    catalog: Res<'w, hollow_engine::blocks::BlockCatalog>,
    usable: Query<'w, 's, &'static BlockState, Without<Dormant>>,
}
impl BlockTargets<'_, '_> {
    pub fn target(&self, position: IVec3) -> Option<Entity> {
        let entity = self.lookup.entity(position)?;
        let state = self.usable.get(entity).ok()?;
        self.catalog
            .interactive(hollow_core::BlockId(state.0.kind().index() as u32))
            .then_some(entity)
    }
}
#[derive(SystemParam)]
pub(crate) struct BlockCollisions<'w, 's> {
    characters: Query<
        'w,
        's,
        (
            &'static CharacterMotion,
            &'static CharacterBody,
            &'static CharacterState,
        ),
        With<GameCharacter>,
    >,
}
impl BlockCollisions<'_, '_> {
    pub fn intersects(&self, position: IVec3, state: VoxelState) -> bool {
        let origin = position.as_dvec3();
        self.characters.iter().any(|(motion, body, character)| {
            let bounds = body.aabb(motion.position, character.pose);
            state.collision_boxes(position).any(|shape| {
                bounds.intersects(&Aabb {
                    min: origin + shape.min.as_dvec3(),
                    max: origin + shape.max.as_dvec3(),
                })
            })
        })
    }
}
pub(crate) fn open_chest(
    event: On<BlockUse>,
    mut commands: Commands,
    chests: Query<&BlockState, (With<Chest>, Without<Dormant>)>,
    topology: ContainerTopology,
    actors: Query<&CharacterMotion>,
    mut next: ResMut<NextState<crate::inventory::InventoryState>>,
) {
    if !chests
        .get(event.entity)
        .is_ok_and(|state| state.0 == event.state)
    {
        return;
    }
    let Ok(motion) = actors.get(event.actor) else {
        return;
    };
    let Some(owner) = topology.canonical(event.entity) else {
        return;
    };
    if topology.owners(owner, motion.position.as_vec3()).is_none() {
        return;
    }
    commands.entity(event.actor).insert(ViewingContainer(owner));
    next.set(crate::inventory::InventoryState::Open);
}
pub(crate) fn open_manual(
    event: On<BlockUse>,
    mut commands: Commands,
    blocks: Query<
        (&BlockEntity, &BlockState, Has<Door>, Has<Gate>),
        (With<ManualOpen>, Without<Dormant>),
    >,
    world: VoxelWorld,
    collisions: BlockCollisions,
    actors: Query<&CharacterMotion>,
) {
    let Ok((block, state, door, gate)) = blocks.get(event.entity) else {
        return;
    };
    if state.0 != event.state
        || world.state(block.position) != Some(state.0)
        || !actors.get(event.actor).is_ok_and(|motion| {
            motion
                .position
                .as_vec3()
                .distance_squared(block.position.as_vec3() + Vec3::splat(0.5))
                <= 25.0
        })
    {
        return;
    }
    let info = state_behavior(state.0);
    let mut next = info.toggle.expect("Manual opening transition");
    if gate && !info.open && event.look.dot(SIDES[info.facing as usize].as_vec3()) < 0.0 {
        next = next.with(&[P::Facing(SIDE_FACING[(info.facing as usize + 2) % 4])]);
    }
    if collisions.intersects(block.position, next) {
        return;
    }
    let mut edits = vec![CheckedEdit {
        position: block.position,
        expected: state.0,
        state: next,
    }];
    if door {
        let position = block.position + if info.upper { IVec3::NEG_Y } else { IVec3::Y };
        let Some(other) = world.state(position) else {
            return;
        };
        if other.kind() != state.0.kind() || state_behavior(other).upper == info.upper {
            return;
        }
        let next = other.with(&[P::Open(!info.open)]);
        if collisions.intersects(position, next) {
            return;
        }
        edits.push(CheckedEdit {
            position,
            expected: other,
            state: next,
        });
    }
    commands.trigger(EditBatch {
        edits,
        consume: None,
        harvest: None,
        tool: None,
    });
}

/// Manual opening is one content plugin using state transitions and block targeting.
pub struct ManualOpeningPlugin;
impl Plugin for ManualOpeningPlugin {
    fn build(&self, app: &mut App) {
        use super::catalog::{Opening, profile};
        let mut registrations = app
            .world_mut()
            .resource_mut::<hollow_engine::blocks::BlockRegistrations>();
        for kind in crate::voxel::VoxelKind::ALL
            .into_iter()
            .filter(|kind| profile(*kind).manual)
        {
            let block = hollow_core::BlockId(kind.index() as u32);
            match profile(kind).opening {
                Opening::Door => registrations.register_bundle::<(ManualOpen, Door)>(block, true),
                Opening::Gate => registrations.register_bundle::<(ManualOpen, Gate)>(block, true),
                _ => registrations.register_bundle::<ManualOpen>(block, true),
            }
        }
        app.add_observer(open_manual);
    }
}
