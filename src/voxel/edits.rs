//! Voxel edit observer, pending edits and border invalidation.

use super::{
    ChunkVoxels, VoxelKind, VoxelState,
    coordinates::{CHUNK_HEIGHT, CHUNK_SIZE, FULL_SKY, split_world_position},
    lighting::LitEdits,
    streaming::{ChunkIndex, PendingEdits, StoredChunks},
};
use bevy::prelude::*;

#[derive(Event, Clone, Copy, Debug)]
pub struct SetVoxel {
    pub world_position: IVec3,
    pub kind: VoxelKind,
    pub state: VoxelState,
}

impl SetVoxel {
    pub const fn with_state(world_position: IVec3, state: VoxelState) -> Self {
        Self {
            world_position,
            kind: state.kind(),
            state,
        }
    }
    pub const fn new(world_position: IVec3, kind: VoxelKind) -> Self {
        Self {
            world_position,
            kind,
            state: kind.default_state(),
        }
    }
}

/// Prioritizes chunks rebuilt because of an edit within the shared work budgets.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct EditedChunk;

pub(crate) fn set_voxel(
    event: On<SetVoxel>,
    mut commands: Commands,
    index: Res<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
    mut pending: ResMut<PendingEdits>,
    mut chunks: Query<&mut ChunkVoxels>,
) {
    apply_voxel(
        *event,
        &mut commands,
        &index,
        &mut stored,
        &mut pending,
        &mut chunks,
        true,
    );
}

#[derive(Event, Clone, Copy)]
pub(crate) struct BlockChanged {
    pub position: IVec3,
    pub old: VoxelState,
    pub new: VoxelState,
}

#[derive(Clone, Copy)]
pub(crate) struct CheckedEdit {
    pub position: IVec3,
    pub expected: VoxelState,
    pub state: VoxelState,
}

/// Gameplay edits are validated together before any state or item is changed.
#[derive(Event)]
pub(crate) struct EditBatch {
    pub edits: Vec<CheckedEdit>,
    pub consume: Option<(Entity, crate::inventory::ItemKind)>,
    pub drop: Option<(crate::inventory::ItemStack, Vec3, Vec3)>,
}

pub(crate) fn edit_batch(
    event: On<EditBatch>,
    mut commands: Commands,
    index: Res<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
    mut pending: ResMut<PendingEdits>,
    mut chunks: Query<&mut ChunkVoxels>,
    mut inventories: Query<(
        &mut crate::inventory::PlayerInventory,
        &crate::inventory::HotbarSelection,
    )>,
) {
    if event.edits.iter().all(|edit| edit.state == edit.expected) {
        return;
    }
    // This is the only ChunkVoxels query: read validation and writes are sequential.
    for (i, edit) in event.edits.iter().enumerate() {
        let (chunk, local) = split_world_position(edit.position);
        if event.edits[..i]
            .iter()
            .any(|other| other.position == edit.position)
            || index
                .get(&chunk)
                .and_then(|&entity| chunks.get(entity).ok())
                .and_then(|voxels| voxels.state(local))
                != Some(edit.expected)
        {
            return;
        }
    }
    if let Some((actor, item)) = event.consume {
        let Ok((mut inventory, selection)) = inventories.get_mut(actor) else {
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
    // Neighbor rules see the complete batch, never an intermediate half-door.
    for edit in &event.edits {
        apply_voxel(
            SetVoxel::with_state(edit.position, edit.state),
            &mut commands,
            &index,
            &mut stored,
            &mut pending,
            &mut chunks,
            false,
        );
    }
    for edit in &event.edits {
        refresh_connections(
            edit.position,
            &index,
            &mut stored,
            &mut chunks,
            &mut commands,
        );
    }
    if let Some((stack, position, velocity)) = event.drop {
        use crate::dropped_item::DroppedItemCommandsExt;
        commands.spawn_dropped_item(stack, position, velocity);
    }
}

fn apply_voxel(
    edit: SetVoxel,
    commands: &mut Commands,
    index: &ChunkIndex,
    stored: &mut StoredChunks,
    pending: &mut PendingEdits,
    chunks: &mut Query<&mut ChunkVoxels>,
    refresh: bool,
) {
    let (chunk, local) = split_world_position(edit.world_position);
    if !(0..CHUNK_HEIGHT).contains(&local.y) {
        return;
    }
    // Read neighbors before borrowing the edited chunk mutably, using the same query.
    // Only removal exposes a formerly opaque cell with stale (usually zero) light.
    let estimate = if edit.kind == VoxelKind::Air
        && let Some(entity) = index.get(&chunk)
        && let Ok(voxels) = chunks.get(*entity)
        && voxels.get(local).is_some_and(VoxelKind::is_opaque)
    {
        let (mut sky, mut block) = (0u8, 0u8);
        for offset in [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ] {
            let position = edit.world_position + offset;
            let light = if position.y >= CHUNK_HEIGHT {
                FULL_SKY
            } else {
                let (neighbor_chunk, neighbor_local) = split_world_position(position);
                let Some(entity) = index.get(&neighbor_chunk) else {
                    continue;
                };
                let Ok(neighbor) = chunks.get(*entity) else {
                    continue;
                };
                if neighbor
                    .get(neighbor_local)
                    .is_none_or(VoxelKind::is_opaque)
                {
                    continue;
                }
                let Some(light) = neighbor.render_light_at(neighbor_local) else {
                    continue;
                };
                light
            };
            let neighbor_sky = light >> 4;
            sky = sky.max(if offset == IVec3::Y && neighbor_sky == 15 {
                15
            } else {
                neighbor_sky.saturating_sub(1)
            });
            block = block.max((light & 15).saturating_sub(1));
        }
        Some((sky << 4) | block)
    } else {
        None
    };
    let old = read_state(edit.world_position, index, stored, chunks);
    let changed = if let Some(entity) = index.get(&chunk) {
        let Ok(mut voxels) = chunks.get_mut(*entity) else {
            return;
        };
        let changed = voxels
            .bypass_change_detection()
            .set_state(local, edit.state);
        if changed {
            if let Some(light) = estimate {
                voxels
                    .bypass_change_detection()
                    .estimate_light(local, light);
            }
            voxels.set_changed();
            let mut entity = commands.entity(*entity);
            entity.try_insert(EditedChunk);
            if !voxels.native_edits.is_empty() {
                entity.try_insert(LitEdits);
            }
        }
        changed
    } else if let Some(voxels) = stored.get_mut(&chunk) {
        voxels.set_state(local, edit.state)
    } else {
        // Applied by `spawn_chunk` once the chunk streams in.
        pending.entry(chunk).or_default().push(edit);
        return;
    };
    if !changed {
        return;
    }
    if let Some(old) = old {
        commands.trigger(BlockChanged {
            position: edit.world_position,
            old,
            new: edit.state,
        });
    }
    if refresh {
        refresh_connections(edit.world_position, index, stored, chunks, commands);
    }

    // Neighbors read this border directly; only their meshes need refreshing.
    for offset in border_neighbors(local) {
        if let Some(entity) = index.get(&(chunk + offset))
            && let Ok(mut voxels) = chunks.get_mut(*entity)
        {
            voxels.mark_sections(ChunkVoxels::sections_around(local.y));
            commands.entity(*entity).try_insert(EditedChunk);
        }
    }
}

/// Neighbor chunks whose meshes sample this cell (faces or corner light).
pub(super) fn border_neighbors(local: IVec3) -> impl Iterator<Item = IVec2> {
    let side = |value: i32| match value {
        0 => -1,
        value if value == CHUNK_SIZE - 1 => 1,
        _ => 0,
    };
    let (x, z) = (side(local.x), side(local.z));
    [
        (z != 0).then_some(IVec2::new(0, z)),
        (x != 0).then_some(IVec2::new(x, 0)),
        (x != 0 && z != 0).then_some(IVec2::new(x, z)),
    ]
    .into_iter()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn border_edits_refresh_cardinal_and_diagonal_neighbors() {
        let corner: Vec<_> = border_neighbors(IVec3::new(0, 5, 15)).collect();
        assert_eq!(corner.len(), 3);
        assert!(corner.contains(&IVec2::new(-1, 0)));
        assert!(corner.contains(&IVec2::new(0, 1)));
        assert!(corner.contains(&IVec2::new(-1, 1)));
        assert_eq!(border_neighbors(IVec3::new(4, 5, 7)).count(), 0);
    }
}

fn read_state(
    position: IVec3,
    index: &ChunkIndex,
    stored: &StoredChunks,
    chunks: &Query<&mut ChunkVoxels>,
) -> Option<VoxelState> {
    let (chunk, local) = split_world_position(position);
    index
        .get(&chunk)
        .and_then(|&entity| chunks.get(entity).ok())
        .or_else(|| stored.get(&chunk))
        .and_then(|voxels| voxels.state(local))
}
fn refresh_connections(
    position: IVec3,
    index: &ChunkIndex,
    stored: &mut StoredChunks,
    chunks: &mut Query<&mut ChunkVoxels>,
    commands: &mut Commands,
) {
    use std::collections::{HashSet, VecDeque};
    const SIDES: [IVec3; 6] = [
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Y,
        IVec3::NEG_Y,
        IVec3::Z,
        IVec3::NEG_Z,
    ];
    let mut queue = VecDeque::new();
    let mut pending = HashSet::new();
    for p in std::iter::once(position).chain(SIDES.map(|d| position + d)) {
        queue.push_back(p);
        pending.insert(p);
    }
    while let Some(position) = queue.pop_front() {
        pending.remove(&position);
        let Some(state) = read_state(position, index, stored, chunks) else {
            continue;
        };
        let next =
            super::rules::connected(position, state, |p| read_state(p, index, stored, chunks));
        if next == state {
            continue;
        }
        let (chunk, local) = split_world_position(position);
        if let Some(&entity) = index.get(&chunk) {
            if let Ok(mut voxels) = chunks.get_mut(entity) {
                voxels.set_state(local, next);
                commands.entity(entity).try_insert((EditedChunk, LitEdits));
            }
        } else if let Some(voxels) = stored.get_mut(&chunk) {
            voxels.set_state(local, next);
        }
        commands.trigger(BlockChanged {
            position,
            old: state,
            new: next,
        });
        for offset in border_neighbors(local) {
            if let Some(&entity) = index.get(&(chunk + offset))
                && let Ok(mut voxels) = chunks.get_mut(entity)
            {
                voxels.mark_sections(ChunkVoxels::sections_around(local.y));
                commands.entity(entity).try_insert(EditedChunk);
            }
        }
        for delta in SIDES {
            let neighbor = position + delta;
            if pending.insert(neighbor) {
                queue.push_back(neighbor);
            }
        }
    }
}

/// A loaded neighbor can restore a connection changed while it was absent.
/// Pumpkin generates matching borders, so only edges shared with an edited
/// chunk are inspected, and only in sections holding connection-bearing blocks.
pub(crate) fn refresh_connection_border(
    added: On<Add<super::VoxelChunk>>,
    mut commands: Commands,
    positions: Query<&super::VoxelChunk>,
    index: Res<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
    mut chunks: Query<&mut ChunkVoxels>,
) {
    let Ok(&super::VoxelChunk { position }) = positions.get(added.entity) else {
        return;
    };
    let Ok(own) = chunks.get(added.entity) else {
        return;
    };
    let own_modified = own.modified;
    let mut seeds = Vec::new();
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
    for seed in seeds {
        refresh_connections(seed, &index, &mut stored, &mut chunks, &mut commands);
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
    for section in 0..super::coordinates::SECTION_COUNT {
        if !voxels
            .states
            .section(section)
            .any(|state| super::rules::has_connections(state.kind()))
        {
            continue;
        }
        let base = section as i32 * super::coordinates::SECTION_HEIGHT;
        for y in base..base + super::coordinates::SECTION_HEIGHT {
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
