//! Local edit-frame light estimates and authoritative snapshot reconciliation.
//! Initial light comes from generation; only edited neighborhoods need relighting.
use super::{ChunkVoxels, VoxelChunk, streaming::ChunkIndex};
use bevy::{
    ecs::system::SystemParam,
    platform::collections::{HashMap, HashSet},
    prelude::*,
    tasks::{Task, futures::check_ready},
};
use hollow_worldgen::reconcile::{LightingChunk, LightingSnapshots};
use hollow_worldgen::sections::Layer;
use std::collections::VecDeque;

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

pub(super) fn edited_lighting(
    event: On<super::edits::BlockChanged>,
    mut version: ResMut<LightingVersion>,
    mut block_light: ResMut<BlockLightEdits>,
) {
    version.0 = version.0.wrapping_add(1);
    let old = event.old.native();
    let new = event.new.native();
    if old.luminance != new.luminance || old.opacity != new.opacity {
        block_light.0.insert(event.position);
    }
}

#[derive(Resource, Default)]
pub(crate) struct BlockLightEdits(HashSet<IVec3>);

/// One mutable voxel query supplies sequential reads and writes during propagation.
#[derive(SystemParam)]
pub(crate) struct BlockLight<'w, 's> {
    index: Res<'w, ChunkIndex>,
    chunks: Query<'w, 's, &'static mut ChunkVoxels>,
    changed: Local<'s, HashMap<IVec2, (u32, bool)>>,
}

impl BlockLight<'_, '_> {
    fn sample(&self, position: IVec3) -> Option<(super::VoxelState, u8)> {
        let (chunk, local) = super::split_world_position(position);
        let voxels = self.chunks.get(*self.index.get(&chunk)?).ok()?;
        Some((voxels.state(local)?, voxels.render_light_at(local)? & 15))
    }

    fn set(&mut self, position: IVec3, light: u8) {
        let (chunk, local) = super::split_world_position(position);
        let Some(index) = hollow_worldgen::coordinates::cell_index(local) else {
            return;
        };
        let Some(&entity) = self.index.get(&chunk) else {
            return;
        };
        let Ok(mut voxels) = self.chunks.get_mut(entity) else {
            return;
        };
        if voxels.render_light().get(index) & 15 != light {
            voxels
                .bypass_change_detection()
                .set_block_light(index, light);
            for offset in std::iter::once(IVec2::ZERO).chain(super::edits::border_neighbors(local))
            {
                let (sections, changed_column) = self.changed.entry(chunk + offset).or_default();
                *sections |= ChunkVoxels::sections_around(local.y);
                *changed_column |= offset == IVec2::ZERO;
            }
        }
    }
}

/// Remove dependent light first, then refill from surviving sources. Increase
/// entries store positions, so a later decrease cannot replay an obsolete level.
pub(crate) fn update_block_light(
    mut commands: Commands,
    mut edits: ResMut<BlockLightEdits>,
    mut light: BlockLight,
    mut decrease: Local<VecDeque<(IVec3, u8)>>,
    mut increase: Local<VecDeque<IVec3>>,
) {
    const SIDES: [IVec3; 6] = [
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Y,
        IVec3::NEG_Y,
        IVec3::Z,
        IVec3::NEG_Z,
    ];
    if edits.0.is_empty() {
        return;
    }
    for position in edits.0.drain() {
        let Some((_, previous)) = light.sample(position) else {
            continue;
        };
        light.set(position, 0);
        decrease.push_back((position, previous));
        increase.push_back(position);
        increase.extend(SIDES.map(|side| position + side));
    }
    while let Some((position, previous)) = decrease.pop_front() {
        for side in SIDES {
            let neighbor = position + side;
            let Some((state, value)) = light.sample(neighbor) else {
                continue;
            };
            if value > 0 && value < previous {
                light.set(neighbor, 0);
                decrease.push_back((neighbor, value));
                if state.native().luminance > 0 {
                    increase.push_back(neighbor);
                }
            } else if value > 0 {
                increase.push_back(neighbor);
            }
        }
    }
    while let Some(position) = increase.pop_front() {
        let Some((state, value)) = light.sample(position) else {
            continue;
        };
        let value = value.max(state.native().luminance);
        light.set(position, value);
        if value <= 1 {
            continue;
        }
        for side in SIDES {
            let neighbor = position + side;
            let Some((state, previous)) = light.sample(neighbor) else {
                continue;
            };
            let propagated = value.saturating_sub(state.native().opacity.max(1));
            if propagated > previous {
                light.set(neighbor, propagated);
                increase.push_back(neighbor);
            }
        }
    }
    // Mark once per affected column, including mesh readers across its borders.
    let changed = std::mem::take(&mut *light.changed);
    for (position, (sections, changed_column)) in &changed {
        if let Some(&entity) = light.index.get(position)
            && let Ok(mut voxels) = light.chunks.get_mut(entity)
        {
            voxels.mark_sections(*sections);
            commands
                .entity(entity)
                .try_insert(super::edits::EditedChunk);
            if *changed_column {
                commands.trigger(crate::block_behavior::visuals::ChunkLightChanged(entity));
            }
        }
    }
    *light.changed = changed;
    light.changed.clear();
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
    context: LightingSnapshots,
    updates: Vec<(IVec2, Layer<u8>)>,
}

#[derive(Resource, Default)]
pub(crate) struct LightingState {
    context: Option<LightingSnapshots>,
    task: Option<Task<Output>>,
    submitted: HashMap<IVec2, Version>,
    ready: HashMap<IVec2, Layer<u8>>,
    synchronized_version: Option<u64>,
}

pub(crate) fn update_lighting(
    mut commands: Commands,
    mut state: ResMut<LightingState>,
    version: Res<LightingVersion>,
    budget: Res<hollow_worldgen::tasks::VoxelTaskBudget>,
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
        // Every input must still identify the same entity and block revision.
        // Discard the context too: reusing its stale light would seed the retry.
        if state
            .submitted
            .iter()
            .any(|(position, &(entity, revision))| {
                index.get(position) != Some(&entity)
                    || chunks
                        .get(entity)
                        .map_or(true, |(_, voxels)| voxels.content_revision != revision)
            })
        {
            state.submitted.clear();
            state.context = None;
            state.ready.clear();
            state.synchronized_version = None;
        } else {
            state.context = Some(output.context);
            state.ready.extend(output.updates);
        }
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
        let states = &voxels.states;
        let version = (entity, voxels.content_revision);
        if state.submitted.get(position) == Some(&version) {
            continue;
        }
        // Only shared snapshots cross the ECS thread; expansion belongs to the worker.
        snapshots.push((
            *position,
            states.clone(),
            voxels.light.clone(),
            voxels.modified,
        ));
        state.submitted.insert(*position, version);
    }
    // Keep only results near unsynchronized edits pending. Unrelated edits and
    // streaming must not postpone light that is already valid for display.
    let invalidated: HashSet<_> = snapshots
        .iter()
        .map(|(position, ..)| *position)
        .chain(removed.iter().copied())
        .flat_map(|position| {
            (-CONTEXT_RADIUS..=CONTEXT_RADIUS).flat_map(move |z| {
                (-CONTEXT_RADIUS..=CONTEXT_RADIUS).map(move |x| position + IVec2::new(x, z))
            })
        })
        .collect();
    let mut changed = Vec::new();
    state.ready.retain(|position, light| {
        if invalidated.contains(position) {
            return true;
        }
        if let Some(&entity) = index.get(position)
            && let Ok((_, mut voxels)) = chunks.get_mut(entity)
        {
            let sections = voxels
                .bypass_change_detection()
                .replace_light(light.clone());
            if sections != 0 {
                voxels.set_changed();
                commands.trigger(crate::block_behavior::visuals::ChunkLightChanged(entity));
                changed.push((*position, sections));
            }
        }
        false
    });
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
    if !snapshots.is_empty() || !removed.is_empty() {
        let context = state.context.take();
        let removed = removed.into_iter().map(|p| (p.x, p.y)).collect::<Vec<_>>();
        state.task =
            Some(
                budget.spawn(hollow_worldgen::tasks::WorkClass::Lighting, 0, async move {
                    let mut context = context.unwrap_or_else(|| {
                        LightingSnapshots::new(
                            super::generation::MIN_Y,
                            hollow_worldgen::coordinates::CHUNK_HEIGHT,
                        )
                    });
                    let snapshots = snapshots
                        .into_iter()
                        .map(|(position, states, light, modified)| LightingChunk {
                            x: position.x,
                            z: position.y,
                            states,
                            light,
                            modified,
                        })
                        .collect::<Vec<_>>();
                    let updates = context
                        .update(snapshots, &removed)
                        .into_iter()
                        .map(|update| (IVec2::new(update.x, update.z), update.light))
                        .collect();
                    Output { context, updates }
                }),
            );
        return;
    }
    state.synchronized_version = Some(version.0);
}
