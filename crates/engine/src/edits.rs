//! Validated terrain mutations with no inventory, loot or game-specific callbacks.
use crate::{chunk::ChunkVoxels, world::ChunkIndex};
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::{prelude::*, system::SystemParam};
use bevy_math::prelude::*;
use bevy_platform::collections::HashMap;
use hollow_core::{
    VoxelCell,
    coordinates::{CHUNK_HEIGHT, CHUNK_SIZE, FULL_SKY, split_world_position},
};

#[derive(Event, Clone, Copy, Debug)]
pub struct SetVoxel<T: VoxelCell> {
    pub world_position: IVec3,
    pub state: T,
}
impl<T: VoxelCell> SetVoxel<T> {
    pub fn with_state(world_position: IVec3, state: T) -> Self {
        Self {
            world_position,
            state,
        }
    }
    pub fn new(world_position: IVec3, kind: T::Kind) -> Self {
        Self::with_state(world_position, T::default_state(kind))
    }
}
#[derive(Event, Clone, Copy, Debug)]
pub struct BlockChanged<T: VoxelCell> {
    pub position: IVec3,
    pub old: T,
    pub new: T,
}
#[derive(Clone, Copy, Debug)]
pub struct CheckedEdit<T: VoxelCell> {
    pub position: IVec3,
    pub expected: T,
    pub state: T,
}
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct EditedChunk;
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct LitEdits;

/// Sparse world coordinates retain only loaded or explicitly retained columns.
#[derive(Resource, Deref, DerefMut)]
pub struct StoredChunks<T: VoxelCell>(HashMap<IVec2, ChunkVoxels<T>>);
#[derive(Resource, Deref, DerefMut)]
pub struct PendingEdits<T: VoxelCell>(HashMap<IVec2, Vec<SetVoxel<T>>>);
impl<T: VoxelCell> Default for StoredChunks<T> {
    fn default() -> Self {
        Self(HashMap::default())
    }
}
impl<T: VoxelCell> Default for PendingEdits<T> {
    fn default() -> Self {
        Self(HashMap::default())
    }
}

#[derive(SystemParam)]
pub struct VoxelEditWorld<'w, 's, T: VoxelCell> {
    index: Res<'w, ChunkIndex>,
    stored: ResMut<'w, StoredChunks<T>>,
    pending: ResMut<'w, PendingEdits<T>>,
    pub chunks: Query<'w, 's, &'static mut ChunkVoxels<T>>,
}

pub fn set_voxel<T: VoxelCell>(
    event: On<SetVoxel<T>>,
    mut commands: Commands,
    mut world: VoxelEditWorld<T>,
) {
    world.apply(*event, &mut commands);
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditRejection {
    Unavailable,
    PreconditionFailed,
    DuplicateCell,
}

impl<T: VoxelCell> VoxelEditWorld<'_, '_, T> {
    pub fn validate(&self, edits: &[CheckedEdit<T>]) -> Result<(), EditRejection> {
        // Placement and flow batches contain only a few cells. Pairwise checks
        // avoid allocating or hashing a coordinate set for each transaction.
        for (index, edit) in edits.iter().enumerate() {
            if edits[..index]
                .iter()
                .any(|other| other.position == edit.position)
            {
                return Err(EditRejection::DuplicateCell);
            }
            let (chunk, local) = split_world_position(edit.position);
            let current = self
                .index
                .get(&chunk)
                .and_then(|&entity| self.chunks.get(entity).ok())
                .and_then(|voxels| voxels.state(local))
                .ok_or(EditRejection::Unavailable)?;
            if current != edit.expected {
                return Err(EditRejection::PreconditionFailed);
            }
        }
        Ok(())
    }
    pub fn commit(
        &mut self,
        edits: &[CheckedEdit<T>],
        commands: &mut Commands,
    ) -> Result<(), EditRejection> {
        self.validate(edits)?;
        self.commit_validated(edits, commands);
        Ok(())
    }

    // The caller must validate before any inventory side effects. No deferred
    // commands run between that validation and these terrain writes.
    pub fn commit_validated(&mut self, edits: &[CheckedEdit<T>], commands: &mut Commands) {
        for edit in edits {
            self.apply(SetVoxel::with_state(edit.position, edit.state), commands);
        }
    }

    pub fn apply(&mut self, edit: SetVoxel<T>, commands: &mut Commands) {
        let (chunk, local) = split_world_position(edit.world_position);
        if !(0..CHUNK_HEIGHT).contains(&local.y) {
            return;
        }
        // Read neighbors before borrowing the edited chunk mutably, using the same query.
        // Only removal exposes a formerly opaque cell with stale (usually zero) light.
        let estimate = if edit.state.is_air()
            && let Some(entity) = self.index.get(&chunk)
            && let Ok(voxels) = self.chunks.get(*entity)
            && voxels.state(local).is_some_and(T::is_opaque)
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
                    if neighbor.state(neighbor_local).is_none_or(T::is_opaque) {
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
        invalidate_border(chunk, local, &self.index, &mut self.chunks, commands);
    }

    pub fn chunk_entity(&self, chunk: IVec2) -> Option<Entity> {
        self.index.get(&chunk).copied()
    }
    pub fn chunk_state(&self, entity: Entity, local: IVec3) -> Option<T> {
        self.chunks.get(entity).ok()?.state(local)
    }
    pub fn state(&self, position: IVec3) -> Option<T> {
        let (chunk, local) = split_world_position(position);
        self.index
            .get(&chunk)
            .and_then(|&entity| self.chunks.get(entity).ok())
            .or_else(|| self.stored.get(&chunk))
            .and_then(|voxels| voxels.state(local))
    }
}

fn invalidate_border<T: VoxelCell>(
    chunk: IVec2,
    local: IVec3,
    index: &ChunkIndex,
    chunks: &mut Query<&mut ChunkVoxels<T>>,
    commands: &mut Commands,
) {
    // Neighbors read this border directly; only their meshes need refreshing.
    for offset in border_neighbors(local) {
        if let Some(&entity) = index.get(&(chunk + offset))
            && let Ok(mut voxels) = chunks.get_mut(entity)
        {
            voxels.mark_sections(ChunkVoxels::<T>::sections_around(local.y));
            commands.entity(entity).try_insert(EditedChunk);
        }
    }
}

/// Neighbor chunks whose meshes sample this cell (faces or corner light).
pub fn border_neighbors(local: IVec3) -> impl Iterator<Item = IVec2> {
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
