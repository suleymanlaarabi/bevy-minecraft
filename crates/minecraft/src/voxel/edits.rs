//! Voxel edit observer, pending edits and border invalidation.

use super::{ChunkVoxels, VoxelState};
use bevy::prelude::*;
use hollow_core::coordinates::CHUNK_SIZE;

pub use hollow_engine::edits::EditedChunk;
pub type SetVoxel = hollow_engine::edits::SetVoxel<VoxelState>;
pub type BlockChanged = hollow_engine::edits::BlockChanged<VoxelState>;
pub type CheckedEdit = hollow_engine::edits::CheckedEdit<VoxelState>;
pub(crate) type VoxelEditWorld<'w, 's> = hollow_engine::edits::VoxelEditWorld<'w, 's, VoxelState>;
pub(crate) use hollow_engine::edits::border_neighbors;

/// Neighbor work belongs to the Minecraft simulation, after a terrain commit.
pub(crate) fn enqueue_neighbors(
    event: On<BlockChanged>,
    mut neighbors: crate::simulation::NeighborUpdates,
) {
    neighbors.enqueue(event.position);
}

/// Gameplay edits are validated together before any state or item is changed.
#[derive(Event)]
pub(crate) struct EditBatch {
    pub edits: Vec<CheckedEdit>,
    pub consume: Option<(Entity, crate::inventory::ItemKind)>,
    pub harvest: Option<crate::block_behavior::loot::Harvest>,
    pub tool: Option<MiningAction>,
}

/// The selected stack is checked again when deferred commands commit.
#[derive(Clone, Copy)]
pub(crate) struct MiningAction {
    pub actor: Entity,
    pub slot: usize,
    pub expected: Option<crate::inventory::ItemStack>,
}

pub(crate) fn edit_batch(
    event: On<EditBatch>,
    mut commands: Commands,
    mut world: VoxelEditWorld,
    mut inventories: Query<(
        &mut crate::inventory::PlayerInventory,
        &crate::inventory::HotbarSelection,
        &crate::character::CharacterState,
    )>,
    mut random: ResMut<crate::block_behavior::loot::LootRandom>,
) {
    if event.edits.iter().all(|edit| edit.state == edit.expected) {
        return;
    }
    // This is the only ChunkVoxels query: read validation and writes are sequential.
    if world.validate(&event.edits).is_err() {
        return;
    }
    let mut mining = None;
    if let Some(tool) = event.tool {
        let Ok((inventory, selection, player)) = inventories.get(tool.actor) else {
            return;
        };
        let Some(harvest) = event.harvest else {
            return;
        };
        if selection.0 != tool.slot || selection.stack(inventory) != tool.expected {
            return;
        }
        let Some(outcome) = crate::inventory::mining::outcome(harvest.state, tool.expected, player)
        else {
            return;
        };
        mining = Some(outcome);
    }
    if let Some(harvest) = event.harvest {
        if !event.edits.iter().any(|edit| {
            edit.position == harvest.position
                && edit.expected == harvest.state
                && edit.state.kind() != edit.expected.kind()
        }) {
            return;
        }
    }
    if let Some((actor, item)) = event.consume {
        let Ok((mut inventory, selection, _)) = inventories.get_mut(actor) else {
            return;
        };
        if selection.stack(&inventory).map(|stack| stack.item()) != Some(item) {
            return;
        }
        let consumed = crate::inventory::PlayerInventory::edit(&mut inventory, |inventory| {
            inventory.consume(selection.0, 1)
        });
        if !consumed {
            return;
        }
    }
    if let (Some(tool), Some(outcome)) = (event.tool, mining) {
        let Ok((mut inventory, _, _)) = inventories.get_mut(tool.actor) else {
            return;
        };
        crate::inventory::PlayerInventory::edit(&mut inventory, |inventory| {
            inventory.damage_tool(tool.slot, outcome.damage)
        });
        if tool.expected.is_some() && inventory.slot(tool.slot).is_none() {
            commands.trigger(crate::audio::Sound::event(
                crate::audio::SoundId::EntityItemBreak,
            ));
        }
    }
    // Emit once per accepted action, including paired blocks and support failures.
    if let Some(edit) = event.edits.first() {
        use crate::audio::{BlockSound, Sound};
        let sound = if let Some(harvest) = event.harvest {
            Some(Sound::block(
                harvest.state.kind(),
                BlockSound::Break,
                harvest.position,
            ))
        } else if event.consume.is_some() {
            Some(Sound::block(
                edit.state.kind(),
                BlockSound::Place,
                edit.position,
            ))
        } else {
            let old = crate::block_behavior::catalog::state_behavior(edit.expected);
            let new = crate::block_behavior::catalog::state_behavior(edit.state);
            (old.open != new.open).then(|| {
                Sound::block(
                    edit.state.kind(),
                    if new.open {
                        BlockSound::Open
                    } else {
                        BlockSound::Close
                    },
                    edit.position,
                )
            })
        };
        if let Some(sound) = sound {
            commands.trigger(sound);
        }
    }
    // Neighbor rules see the complete batch, never an intermediate half-door.
    if let Some(tool) = event.tool {
        commands.trigger(crate::survival::Exhaustion {
            entity: tool.actor,
            amount: 0.005,
        });
    }
    world.commit_validated(&event.edits, &mut commands);
    if let Some(harvest) = event.harvest
        && mining.is_none_or(|outcome| outcome.can_harvest)
    {
        use crate::dropped_item::DroppedItemCommandsExt;
        for stack in crate::block_behavior::loot::drops(
            harvest.state,
            mining.is_some_and(|outcome| outcome.shears),
            &mut random,
        )
        .into_iter()
        .flatten()
        {
            commands.spawn_dropped_item(
                stack,
                harvest.position.as_vec3() + Vec3::splat(0.5) + harvest.offset,
                harvest.velocity,
            );
        }
    }
}

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct ConnectionBorders(std::collections::VecDeque<Entity>);
pub(crate) fn queue_connection_border(
    event: On<Add<super::VoxelChunk>>,
    mut pending: ResMut<ConnectionBorders>,
) {
    pending.push_back(event.entity);
}

/// A loaded neighbor can restore a connection changed while it was absent.
/// Pumpkin generates matching borders, so only edges shared with an edited
/// chunk are inspected, and only in sections holding connection-bearing blocks.
pub(crate) fn refresh_connection_border(
    mut pending: ResMut<ConnectionBorders>,
    mut neighbors: crate::simulation::NeighborUpdates,
    positions: Query<&super::VoxelChunk>,
    index: Res<super::streaming::ChunkIndex>,
    chunks: Query<&ChunkVoxels>,
    mut seeds: Local<Vec<IVec3>>,
) {
    // Border discovery is preparation, outside observers. At most one arrival
    // is scanned per frame; the resulting cells use the persistent fixed budget.
    let Some(entity) = pending.pop_front() else {
        return;
    };
    let Ok(&super::VoxelChunk { position }) = positions.get(entity) else {
        return;
    };
    let Ok(own) = chunks.get(entity) else {
        return;
    };
    let own_modified = own.modified;
    seeds.clear();
    // (neighbor offset, horizontal axis, own edge, neighbor edge)
    for (offset, axis, own_edge, neighbor_edge) in [
        (IVec2::NEG_X, 0, 0, CHUNK_SIZE - 1),
        (IVec2::X, 0, CHUNK_SIZE - 1, 0),
        (IVec2::NEG_Y, 2, 0, CHUNK_SIZE - 1),
        (IVec2::Y, 2, CHUNK_SIZE - 1, 0),
    ] {
        let neighbor_position = position + offset;
        let Some(neighbor) = index
            .get(&neighbor_position)
            .and_then(|&entity| chunks.get(entity).ok())
        else {
            continue;
        };
        if !own_modified && !neighbor.modified {
            continue;
        }
        for (voxels, chunk, edge) in [
            (own, position, own_edge),
            (neighbor, neighbor_position, neighbor_edge),
        ] {
            collect_border_connections(voxels, chunk, axis, edge, &mut seeds);
        }
    }
    for seed in seeds.drain(..) {
        neighbors.enqueue(seed);
    }
}

fn collect_border_connections(
    voxels: &ChunkVoxels,
    chunk: IVec2,
    axis: usize,
    edge: i32,
    seeds: &mut Vec<IVec3>,
) {
    let origin = IVec3::new(chunk.x * CHUNK_SIZE, 0, chunk.y * CHUNK_SIZE);
    for section in 0..hollow_core::coordinates::SECTION_COUNT {
        if !voxels
            .states
            .section(section)
            .any(|state| super::rules::has_connections(state.kind()))
        {
            continue;
        }
        let base = section as i32 * hollow_core::coordinates::SECTION_HEIGHT;
        for y in base..base + hollow_core::coordinates::SECTION_HEIGHT {
            for along in 0..CHUNK_SIZE {
                let mut local = IVec3::new(along, y, along);
                local[axis] = edge;
                if voxels.get(local).is_some_and(super::rules::has_connections) {
                    seeds.push(origin + local);
                }
            }
        }
    }
}
