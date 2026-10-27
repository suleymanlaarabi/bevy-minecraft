//! Voxel edit observer, pending edits and border invalidation.

use super::{
    ChunkVoxels, VoxelKind, VoxelState,
    lighting::LitEdits,
    streaming::{ChunkIndex, PendingEdits, StoredChunks},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_worldgen::coordinates::{CHUNK_HEIGHT, CHUNK_SIZE, FULL_SKY, split_world_position};

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

/// One mutable voxel query serves validation, edits, and connection propagation.
#[derive(SystemParam)]
pub(crate) struct VoxelEditWorld<'w, 's> {
    index: Res<'w, ChunkIndex>,
    stored: ResMut<'w, StoredChunks>,
    pending: ResMut<'w, PendingEdits>,
    chunks: Query<'w, 's, &'static mut ChunkVoxels>,
}

pub(crate) fn set_voxel(event: On<SetVoxel>, mut commands: Commands, mut world: VoxelEditWorld) {
    world.apply(*event, &mut commands, true);
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
    if !world.validate(&event.edits) {
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

impl VoxelEditWorld<'_, '_> {
    pub(crate) fn validate(&self, edits: &[CheckedEdit]) -> bool {
        edits.iter().enumerate().all(|(i, edit)| {
            !edits[..i]
                .iter()
                .any(|other| other.position == edit.position)
                && {
                    let (chunk, local) = split_world_position(edit.position);
                    self.index
                        .get(&chunk)
                        .and_then(|&entity| self.chunks.get(entity).ok())
                        .and_then(|voxels| voxels.state(local))
                        == Some(edit.expected)
                }
        })
    }
    pub(crate) fn commit(&mut self, edits: &[CheckedEdit], commands: &mut Commands) -> bool {
        if !self.validate(edits) {
            return false;
        }
        self.commit_validated(edits, commands);
        true
    }

    // The caller must validate before any inventory side effects. No deferred
    // commands run between that validation and these terrain writes.
    fn commit_validated(&mut self, edits: &[CheckedEdit], commands: &mut Commands) {
        for edit in edits {
            self.apply(
                SetVoxel::with_state(edit.position, edit.state),
                commands,
                false,
            );
        }
        for edit in edits {
            self.refresh_connections(edit.position, commands);
        }
    }

    fn apply(&mut self, edit: SetVoxel, commands: &mut Commands, refresh: bool) {
        let (chunk, local) = split_world_position(edit.world_position);
        if !(0..CHUNK_HEIGHT).contains(&local.y) {
            return;
        }
        // Read neighbors before borrowing the edited chunk mutably, using the same query.
        // Only removal exposes a formerly opaque cell with stale (usually zero) light.
        let estimate = if edit.kind == VoxelKind::Air
            && let Some(entity) = self.index.get(&chunk)
            && let Ok(voxels) = self.chunks.get(*entity)
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
                    let Some(entity) = self.index.get(&neighbor_chunk) else {
                        continue;
                    };
                    let Ok(neighbor) = self.chunks.get(*entity) else {
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
        let old = self.state(edit.world_position);
        let changed = if let Some(entity) = self.index.get(&chunk) {
            let Ok(mut voxels) = self.chunks.get_mut(*entity) else {
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
                if voxels.modified {
                    entity.try_insert(LitEdits);
                }
            }
            changed
        } else if let Some(voxels) = self.stored.get_mut(&chunk) {
            voxels.set_state(local, edit.state)
        } else {
            // Applied by `spawn_chunk` once the chunk streams in.
            self.pending.entry(chunk).or_default().push(edit);
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
            self.refresh_connections(edit.world_position, commands);
        }

        invalidate_border(chunk, local, &self.index, &mut self.chunks, commands);
    }

    pub(crate) fn chunk_entity(&self, chunk: IVec2) -> Option<Entity> {
        self.index.get(&chunk).copied()
    }
    pub(crate) fn chunk_state(&self, entity: Entity, local: IVec3) -> Option<VoxelState> {
        self.chunks.get(entity).ok()?.state(local)
    }
    pub(crate) fn state(&self, position: IVec3) -> Option<VoxelState> {
        let (chunk, local) = split_world_position(position);
        self.index
            .get(&chunk)
            .and_then(|&entity| self.chunks.get(entity).ok())
            .or_else(|| self.stored.get(&chunk))
            .and_then(|voxels| voxels.state(local))
    }

    fn refresh_connections(&mut self, position: IVec3, commands: &mut Commands) {
        commands
            .queue(move |world: &mut World| crate::simulation::enqueue_neighbors(world, position));
    }
}

fn invalidate_border(
    chunk: IVec2,
    local: IVec3,
    index: &ChunkIndex,
    chunks: &mut Query<&mut ChunkVoxels>,
    commands: &mut Commands,
) {
    // Neighbors read this border directly; only their meshes need refreshing.
    for offset in border_neighbors(local) {
        if let Some(&entity) = index.get(&(chunk + offset))
            && let Ok(mut voxels) = chunks.get_mut(entity)
        {
            voxels.mark_sections(ChunkVoxels::sections_around(local.y));
            commands.entity(entity).try_insert(EditedChunk);
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
    mut commands: Commands,
    positions: Query<&super::VoxelChunk>,
    mut world: VoxelEditWorld,
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
    let Ok(own) = world.chunks.get(entity) else {
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
        let Some(neighbor) = world
            .index
            .get(&neighbor_position)
            .and_then(|&entity| world.chunks.get(entity).ok())
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
        world.refresh_connections(seed, &mut commands);
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
    for section in 0..hollow_worldgen::coordinates::SECTION_COUNT {
        if !voxels
            .states
            .section(section)
            .any(|state| super::rules::has_connections(state.kind()))
        {
            continue;
        }
        let base = section as i32 * hollow_worldgen::coordinates::SECTION_HEIGHT;
        for y in base..base + hollow_worldgen::coordinates::SECTION_HEIGHT {
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
