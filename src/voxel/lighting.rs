//! Serialized Pumpkin updates run off the ECS thread; voxel edits remain immediate.
use super::{ChunkVoxels, VoxelChunk, streaming::ChunkIndex};
use bevy::{
    platform::collections::HashMap,
    prelude::*,
    tasks::{Task, TaskPool, TaskPoolBuilder, futures::check_ready},
};
use pumpkin_worldgen::{Dimension, LightingChunk, LightingUpdate, LightingWorld};
use std::sync::{Arc, OnceLock};

type Version = (Entity, u64);
struct Output {
    context: LightingWorld,
    updates: Vec<LightingUpdate>,
}

#[derive(Resource, Default)]
pub(crate) struct LightingState {
    context: Option<LightingWorld>,
    task: Option<Task<Output>>,
    submitted: HashMap<IVec2, Version>,
    ready: HashMap<IVec2, Arc<[u8]>>,
}

fn pool() -> &'static TaskPool {
    static POOL: OnceLock<TaskPool> = OnceLock::new();
    POOL.get_or_init(|| {
        TaskPoolBuilder::new()
            .num_threads(1)
            .thread_name("voxel-lighting".into())
            .build()
    })
}

pub(crate) fn update_lighting(
    mut state: ResMut<LightingState>,
    index: Res<ChunkIndex>,
    mut chunks: Query<(Entity, &VoxelChunk, &mut ChunkVoxels)>,
) {
    if let Some(task) = state.task.as_mut() {
        let Some(output) = check_ready(task) else {
            return;
        };
        state.task = None;
        state.context = Some(output.context);
        for update in output.updates {
            state
                .ready
                .insert(IVec2::new(update.x, update.z), update.light.into());
        }
    }
    let removed: Vec<_> = state
        .submitted
        .keys()
        .filter(|p| !index.contains_key(*p))
        .copied()
        .collect();
    for position in &removed {
        state.submitted.remove(position);
        state.ready.remove(position);
    }
    sync_borders(&index, &mut chunks, &removed);
    let mut snapshots = Vec::new();
    for (entity, chunk, voxels) in &chunks {
        let Some(states) = &voxels.native_states else {
            continue;
        };
        let version = (entity, voxels.content_revision);
        if state.submitted.get(&chunk.position) == Some(&version) {
            continue;
        }
        snapshots.push(LightingChunk {
            x: chunk.position.x,
            z: chunk.position.y,
            states: states.clone(),
            light: voxels.light.clone(),
            edits: voxels.native_edits.iter().copied().collect(),
        });
        state.submitted.insert(chunk.position, version);
    }
    if !snapshots.is_empty() || !removed.is_empty() {
        let context = state.context.take();
        let removed = removed.into_iter().map(|p| (p.x, p.y)).collect::<Vec<_>>();
        state.task = Some(pool().spawn(async move {
            let mut context = context.unwrap_or_else(|| LightingWorld::new(Dimension::Overworld));
            let updates = context.update(snapshots, &removed);
            Output { context, updates }
        }));
        return;
    }
    // The worker now represents the current entities AND their current native
    // revisions. Retain intermediate results until then, so stale completions
    // cannot erase newer edits and dependent neighbor light cannot get lost.
    let mut changed = Vec::new();
    for (position, light) in state.ready.drain() {
        if let Some(&entity) = index.get(&position)
            && let Ok((_, _, mut voxels)) = chunks.get_mut(entity)
            && voxels.bypass_change_detection().replace_light(light)
        {
            voxels.set_changed();
            changed.push(position);
        }
    }
    // Center light and all eight border/corner samples are refreshed together.
    sync_borders(&index, &mut chunks, &changed);
}

fn sync_borders(
    index: &ChunkIndex,
    chunks: &mut Query<(Entity, &VoxelChunk, &mut ChunkVoxels)>,
    changed: &[IVec2],
) {
    let mut affected = bevy::platform::collections::HashSet::new();
    for &position in changed {
        for z in -1..=1 {
            for x in -1..=1 {
                affected.insert(position + IVec2::new(x, z));
            }
        }
    }
    for position in affected {
        let Some(&entity) = index.get(&position) else {
            continue;
        };
        for z in -1..=1 {
            for x in -1..=1 {
                if x == 0 && z == 0 {
                    continue;
                }
                let direction = IVec2::new(x, z);
                let neighbor = index
                    .get(&(position + direction))
                    .and_then(|&e| chunks.get(e).ok())
                    .map(|(_, _, v)| v.clone());
                if let Ok((_, _, mut voxels)) = chunks.get_mut(entity)
                    && voxels
                        .bypass_change_detection()
                        .sync_light_halo(direction, neighbor.as_ref())
                {
                    voxels.set_changed();
                }
            }
        }
    }
}

/// Refresh corners before building meshes for additions and player edits.
pub(crate) fn initialize_light_borders(
    index: Res<ChunkIndex>,
    mut chunks: Query<(Entity, &VoxelChunk, &mut ChunkVoxels)>,
) {
    // Change-detection filters also access their component. Inspect Mut's change
    // flag through this single query rather than adding a conflicting query.
    let changed = chunks
        .iter_mut()
        .filter_map(|(_, chunk, voxels)| voxels.is_changed().then_some(chunk.position))
        .collect::<Vec<_>>();
    sync_borders(&index, &mut chunks, &changed);
}
