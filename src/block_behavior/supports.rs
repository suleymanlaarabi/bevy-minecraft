//! Bounded entity work; dependency and persistence state stay on the owning entity.
use super::catalog::{SIDE_FACING, SIDES, SupportRule, profile, state_behavior};
use super::containers::{BlockEntity, Door, Dormant, Vine, WallSupport};
use crate::voxel::{
    VoxelKind, VoxelWorld,
    edits::{BlockChanged, CheckedEdit, EditBatch},
};
use bevy::{ecs::entity::EntityIndexSet, prelude::*};

pub(crate) type SupportQueue = super::queue::EntityWork<SupportCheck>;
#[derive(Component, Default)]
#[component(storage = "SparseSet", on_add = super::queue::enqueue::<Self>)]
pub(crate) struct SupportCheck {
    waiting: Option<IVec2>,
    complete: bool,
    edit: Option<EditBatch>,
}

fn schedule(world: &mut World, position: IVec3, create: bool) {
    if !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&position.y) {
        return;
    }
    let existing = super::containers::find_block(world, position);
    let entity = match existing {
        Some(entity) => entity,
        None if create => super::containers::block(world, position),
        None => return,
    };
    if let Some(mut check) = world.get_mut::<SupportCheck>(entity) {
        if check.waiting.take().is_none() {
            return;
        }
        world.resource_mut::<SupportQueue>().push_back(entity);
    } else {
        world.entity_mut(entity).insert(SupportCheck::default());
    }
}
pub(crate) fn changed(event: On<BlockChanged>, mut commands: Commands) {
    let position = event.position;
    let create = profile(event.new.kind()).support != SupportRule::None;
    commands.queue(move |world: &mut World| {
        schedule(world, position, create);
        for delta in SIDES.into_iter().chain([IVec3::Y, IVec3::NEG_Y]) {
            schedule(world, position + delta, false);
        }
    });
}
pub(crate) fn arrived(
    event: On<Add<crate::voxel::VoxelChunk>>,
    chunks: Query<&crate::voxel::VoxelChunk>,
    mut checks: Query<(Entity, &mut SupportCheck)>,
    mut queue: ResMut<SupportQueue>,
) {
    let Ok(chunk) = chunks.get(event.entity) else {
        return;
    };
    for (entity, mut check) in &mut checks {
        if check.waiting == Some(chunk.position) {
            check.waiting = None;
            queue.push_back(entity);
        }
    }
}
pub(crate) fn update(
    mut commands: Commands,
    world: VoxelWorld,
    mut queue: ResMut<SupportQueue>,
    mut entities: Local<EntityIndexSet>,
    mut checks: Query<(
        &BlockEntity,
        &mut SupportCheck,
        Has<Door>,
        Has<WallSupport>,
        Has<Vine>,
        Has<Dormant>,
    )>,
) {
    use pumpkin_data::BlockDirection;
    let count = queue.len().min(256);
    entities.clear();
    entities.extend(queue.drain(..count));
    checks
        .par_iter_many_unique_mut(&*entities)
        .for_each(|result| {
            let Ok((block, mut check, door, wall, vine, dormant)) = result else {
                return;
            };
            let position = block.position;
            if dormant {
                check.waiting = Some(crate::voxel::split_world_position(position).0);
                return;
            }
            let Some(state) = world.state(position) else {
                check.waiting = Some(crate::voxel::split_world_position(position).0);
                return;
            };
            check.complete = false;
            if !door && !wall && !vine {
                check.complete = true;
                return;
            }
            let info = state_behavior(state);
            let mut unavailable = false;
            let mut sample = |p: IVec3| {
                if !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&p.y) {
                    return Some(VoxelKind::Air.default_state());
                }
                let value = world.state(p);
                if value.is_none() {
                    unavailable = true;
                    check.waiting = Some(crate::voxel::split_world_position(p).0);
                }
                value
            };
            let mut next = state;
            let mut edits = Vec::new();
            let mut drop_item = false;
            match (door, wall, vine) {
                (true, _, _) => {
                    let other_position =
                        position + if info.upper { IVec3::NEG_Y } else { IVec3::Y };
                    let other = sample(other_position);
                    let paired = other.is_some_and(|other| {
                        other.kind() == state.kind() && state_behavior(other).upper != info.upper
                    });
                    let supported = info.upper
                        || sample(position - IVec3::Y)
                            .is_some_and(|s| s.native().is_side_solid(BlockDirection::Up));
                    if !paired || !supported {
                        next = state.removed();
                        drop_item = !info.upper;
                        if paired && let Some(other) = other {
                            edits.push(CheckedEdit {
                                position: other_position,
                                expected: other,
                                state: other.removed(),
                            });
                        }
                    }
                }
                (_, true, _) => {
                    let direction = SIDES[info.facing as usize];
                    let face = [
                        BlockDirection::North,
                        BlockDirection::East,
                        BlockDirection::South,
                        BlockDirection::West,
                    ][info.facing as usize];
                    if !sample(position - direction).is_some_and(|s| s.native().is_side_solid(face))
                    {
                        next = state.removed();
                        drop_item = true;
                    }
                }
                (_, _, true) => {
                    let mut surviving = info.attachments;
                    for (i, delta) in SIDES.into_iter().chain([IVec3::Y]).enumerate() {
                        if surviving & (1 << i) == 0 {
                            continue;
                        }
                        let face = [
                            BlockDirection::South,
                            BlockDirection::West,
                            BlockDirection::North,
                            BlockDirection::East,
                            BlockDirection::Down,
                        ][i];
                        let attached = sample(position + delta)
                            .is_some_and(|s| s.native().is_side_solid(face));
                        let inherited = i < 4
                            && sample(position + IVec3::Y).is_some_and(|s| {
                                s.kind() == state.kind()
                                    && state_behavior(s).attachments & (1 << i) != 0
                            });
                        if !attached && !inherited {
                            surviving &= !(1 << i);
                            let side = if i == 4 {
                                pumpkin_data::block_properties::Facing::Up
                            } else {
                                SIDE_FACING[i]
                            };
                            next = next.with(&[crate::voxel::VoxelState::attachment(side, false)]);
                        }
                    }
                    if surviving == 0 {
                        next = state.removed();
                        drop_item = true;
                    }
                }
                _ => unreachable!("Support capability checked above"),
            }
            if unavailable {
                return;
            }
            check.complete = true;
            if next == state {
                return;
            }
            edits.push(CheckedEdit {
                position,
                expected: state,
                state: next,
            });
            check.edit = Some(EditBatch {
                edits,
                consume: None,
                harvest: drop_item.then_some(super::loot::Harvest {
                    position,
                    state,
                    offset: Vec3::ZERO,
                    velocity: Vec3::Y * 1.2,
                }),
                tool: None,
            });
        });
    // Complete the entire batch before any commit can invalidate another candidate.
    for &entity in entities.iter() {
        let Ok((block, check, _, _, _, _)) = checks.get(entity) else {
            continue;
        };
        if !check.complete {
            continue;
        }
        commands.entity(entity).remove::<SupportCheck>();
        if world
            .state(block.position)
            .is_some_and(|state| !super::containers::materialized(state.kind()))
        {
            commands.entity(entity).despawn();
        }
    }
    // Deferred removals have not run yet: take proposals in the original FIFO order.
    for &entity in entities.iter() {
        if let Ok((_, mut check, _, _, _, _)) = checks.get_mut(entity)
            && let Some(edit) = check.edit.take()
        {
            commands.trigger(edit);
        }
    }
}

pub(crate) fn restore(session: Option<Res<crate::world::WorldSession>>, mut commands: Commands) {
    if let Some(session) = session {
        for &position in &session.resume.support_pending {
            commands
                .queue(move |world: &mut World| schedule(world, IVec3::from_array(position), true));
        }
    }
}
