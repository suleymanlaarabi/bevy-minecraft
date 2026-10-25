//! Serialized Pumpkin light updates run off the ECS thread; voxel edits remain immediate.
//!
//! Streamed chunks keep the light Pumpkin computed during generation. Only the
//! neighborhood of edited chunks is mirrored into the lighting context.
use super::{ChunkVoxels, VoxelChunk, storage::Layer, streaming::ChunkIndex};
use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
    tasks::{Task, TaskPool, TaskPoolBuilder, futures::check_ready},
};
use pumpkin_worldgen::{Dimension, LightingChunk, LightingWorld};
use std::sync::OnceLock;

/// Chebyshev radius around an edited chunk mirrored into the lighting context: the
/// relit 3x3 area plus the ring that seeds light across its border.
const CONTEXT_RADIUS: i32 = 2;

/// Chunks with native edits, the only ones whose neighborhood is mirrored into the
/// lighting context. Edits are never forgotten, so the marker stays until despawn.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct LitEdits;

type Version = (Entity, u64);

/// Content/arrival notifications let synchronized lighting sleep completely.
#[derive(Resource, Default)]
pub(super) struct LightingVersion(u64);

pub(super) fn edited_lighting(_event: On<Insert<LitEdits>>, mut version: ResMut<LightingVersion>) {
    version.0 = version.0.wrapping_add(1);
}

pub(super) fn added_lighting(_event: On<Add<VoxelChunk>>, mut version: ResMut<LightingVersion>) {
    version.0 = version.0.wrapping_add(1);
}

pub(super) fn removed_lighting(
    _event: On<Remove<VoxelChunk>>,
    mut version: ResMut<LightingVersion>,
) {
    version.0 = version.0.wrapping_add(1);
}

struct Output {
    context: LightingWorld,
    updates: Vec<(IVec2, Layer<u8>)>,
}

#[derive(Resource, Default)]
pub(crate) struct LightingState {
    context: Option<LightingWorld>,
    task: Option<Task<Output>>,
    submitted: HashMap<IVec2, Version>,
    ready: HashMap<IVec2, Layer<u8>>,
    synchronized_version: Option<u64>,
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
    version: Res<LightingVersion>,
    index: Res<ChunkIndex>,
    edited: Query<&VoxelChunk, With<LitEdits>>,
    mut chunks: Query<(Entity, &mut ChunkVoxels)>,
    mut relevant: Local<HashSet<IVec2>>,
    mut edited_positions: Local<Vec<IVec2>>,
    mut current_positions: Local<Vec<IVec2>>,
) {
    if state.task.is_none()
        && state.ready.is_empty()
        && state.synchronized_version == Some(version.0)
    {
        return;
    }
    if let Some(task) = state.task.as_mut() {
        let Some(output) = check_ready(task) else {
            return;
        };
        state.task = None;
        state.context = Some(output.context);
        state.ready.extend(output.updates);
    }
    current_positions.clear();
    current_positions.extend(edited.iter().map(|chunk| chunk.position));
    current_positions.sort_unstable_by_key(|position| (position.x, position.y));
    if *current_positions != *edited_positions {
        std::mem::swap(&mut *current_positions, &mut *edited_positions);
        relevant.clear();
        for &position in edited_positions.iter() {
            for z in -CONTEXT_RADIUS..=CONTEXT_RADIUS {
                for x in -CONTEXT_RADIUS..=CONTEXT_RADIUS {
                    relevant.insert(position + IVec2::new(x, z));
                }
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
        let Ok((entity, voxels)) = chunks.get(entity) else {
            continue;
        };
        let Some(states) = &voxels.native_states else {
            continue;
        };
        let version = (entity, voxels.content_revision);
        if state.submitted.get(position) == Some(&version) {
            continue;
        }
        // Only shared snapshots cross the ECS thread; expansion belongs to the worker.
        snapshots.push((
            *position,
            states.clone(),
            voxels.light.clone(),
            voxels.native_edits.clone(),
        ));
        state.submitted.insert(*position, version);
    }
    if !snapshots.is_empty() || !removed.is_empty() {
        let context = state.context.take();
        let removed = removed.into_iter().map(|p| (p.x, p.y)).collect::<Vec<_>>();
        state.task = Some(pool().spawn(async move {
            let mut context = context.unwrap_or_else(|| LightingWorld::new(Dimension::Overworld));
            let snapshots = snapshots
                .into_iter()
                .map(|(position, states, light, edits)| LightingChunk {
                    x: position.x,
                    z: position.y,
                    states: states.to_cells(),
                    light: light.to_cells(),
                    edits: edits.iter().copied().collect(),
                })
                .collect::<Vec<_>>();
            let updates = context
                .update(snapshots, &removed)
                .into_iter()
                .map(|update| {
                    (
                        IVec2::new(update.x, update.z),
                        Layer::from_cells(&update.light),
                    )
                })
                .collect();
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
            && let Ok((_, mut voxels)) = chunks.get_mut(entity)
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
                    && let Ok((_, mut voxels)) = chunks.get_mut(entity)
                {
                    voxels.mark_sections(sections);
                }
            }
        }
    }
    state.synchronized_version = Some(version.0);
}
