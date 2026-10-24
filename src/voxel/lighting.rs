//! Serialized Pumpkin light updates run off the ECS thread; voxel edits remain immediate.
//!
//! Streamed chunks keep the light Pumpkin computed during generation. Only the
//! neighborhood of edited chunks is mirrored into the lighting context.
use super::{ChunkVoxels, VoxelChunk, streaming::ChunkIndex};
use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
    tasks::{Task, TaskPool, TaskPoolBuilder, futures::check_ready},
};
use pumpkin_worldgen::{Dimension, LightingChunk, LightingUpdate, LightingWorld};
use std::sync::{Arc, OnceLock};

/// Chebyshev radius around an edited chunk mirrored into the lighting context: the
/// relit 3x3 area plus the ring that seeds light across its border.
const CONTEXT_RADIUS: i32 = 2;

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
    mut relevant: Local<HashSet<IVec2>>,
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
    relevant.clear();
    for (_, chunk, voxels) in &chunks {
        if voxels.native_edits.is_empty() {
            continue;
        }
        for z in -CONTEXT_RADIUS..=CONTEXT_RADIUS {
            for x in -CONTEXT_RADIUS..=CONTEXT_RADIUS {
                relevant.insert(chunk.position + IVec2::new(x, z));
            }
        }
    }
    let removed: Vec<_> = state
        .submitted
        .keys()
        .filter(|p| !index.contains_key(*p) || !relevant.contains(*p))
        .copied()
        .collect();
    for position in &removed {
        state.submitted.remove(position);
        state.ready.remove(position);
    }
    let mut snapshots = Vec::new();
    for position in relevant.iter() {
        let Some(&entity) = index.get(position) else {
            continue;
        };
        let Ok((entity, _, voxels)) = chunks.get(entity) else {
            continue;
        };
        let Some(states) = &voxels.native_states else {
            continue;
        };
        let version = (entity, voxels.content_revision);
        if state.submitted.get(position) == Some(&version) {
            continue;
        }
        snapshots.push(LightingChunk {
            x: position.x,
            z: position.y,
            states: states.clone(),
            light: voxels.light.clone(),
            edits: voxels.native_edits.iter().copied().collect(),
        });
        state.submitted.insert(*position, version);
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
        {
            let sections = voxels.bypass_change_detection().replace_light(light);
            if sections != 0 {
                voxels.set_changed();
                changed.push((position, sections));
            }
        }
    }
    // Neighbors sample this light at their border face corners.
    for (position, sections) in changed {
        for z in -1..=1 {
            for x in -1..=1 {
                if (x, z) == (0, 0) {
                    continue;
                }
                if let Some(&entity) = index.get(&(position + IVec2::new(x, z)))
                    && let Ok((_, _, mut voxels)) = chunks.get_mut(entity)
                {
                    voxels.mark_sections(sections);
                }
            }
        }
    }
}
