//! Local edit-frame light estimates and authoritative snapshot reconciliation.
//! Initial light comes from generation; only edited neighborhoods need relighting.
use super::{LightingChunk, LightingSnapshots};
use crate::{
    chunk::{ChunkVoxels, VoxelChunk},
    content::RegisteredContent,
    world::{ChunkIndex, WorldLayout},
};
use bevy_ecs::prelude::*;
use bevy_math::{IVec2, IVec3};
use bevy_platform::collections::{HashMap, HashSet};
use bevy_tasks::{Task, futures::check_ready};
use hollow_core::sections::Layer;
use hollow_core::{StateId, VoxelCell};

/// Chebyshev radius around an edited chunk mirrored into the lighting context: the
/// relit 3x3 area plus the ring that seeds light across its border.
const CONTEXT_RADIUS: i32 = 2;

pub use crate::edits::LitEdits;

type Version = (Entity, u64);

/// Content/arrival notifications let synchronized lighting sleep completely.
#[derive(Resource, Default)]
pub struct LightingVersion(u64);

pub fn edited_lighting<T: VoxelCell>(
    event: On<crate::edits::BlockChanged<T>>,
    content: Res<RegisteredContent>,
    mut version: ResMut<LightingVersion>,
    mut block_light: ResMut<BlockLightEdits>,
    mut state: ResMut<LightingState<T>>,
    layout: Res<WorldLayout>,
) {
    version.0 = version.0.wrapping_add(1);
    let old = content.state(StateId(event.old.id()));
    let new = content.state(StateId(event.new.id()));
    let (chunk, local) = layout.split(event.position);
    let journal = state.changes.entry(chunk).or_insert_with(|| LightEdits {
        count: 0,
        cells: Some(Vec::new()),
    });
    journal.count = journal.count.saturating_add(1);
    if old.luminance != new.luminance
        || old.opacity != new.opacity
        || old.can_occlude() != new.can_occlude()
        || old.is_solid_render() != new.is_solid_render()
    {
        if let Some(entries) = &mut journal.cells {
            if entries.len() < 4096 {
                if let Some(index) =
                    hollow_core::coordinates::column_cell_index(local, layout.section_count())
                {
                    entries.push(index as u32);
                }
            } else {
                journal.cells = None;
            }
        }
    }
    if old.luminance != new.luminance || old.opacity != new.opacity {
        block_light.0.insert(event.position);
    }
}

#[derive(Resource, Default)]
pub struct BlockLightEdits(HashSet<IVec3>);

/// Both lighting systems mutate `ChunkVoxels`: run conditions keep idle frames from
/// scheduling them against every voxel reader.
pub fn block_light_pending(edits: Res<BlockLightEdits>) -> bool {
    !edits.0.is_empty()
}
pub fn lighting_pending<T: VoxelCell>(
    state: Res<LightingState<T>>,
    version: Res<LightingVersion>,
) -> bool {
    state.task.is_some() || !state.ready.is_empty() || state.synchronized_version != Some(version.0)
}

/// Resolve the edited neighborhoods once; propagation then uses indexed cells and links.
pub fn update_block_light<T: VoxelCell>(
    mut commands: Commands,
    mut edits: ResMut<BlockLightEdits>,
    layout: Res<WorldLayout>,
    content: Res<RegisteredContent>,
    index: Res<ChunkIndex>,
    mut chunks: Query<&mut ChunkVoxels<T>>,
    mut relevant: Local<HashSet<IVec2>>,
    mut changed: Local<HashMap<IVec2, (u32, bool)>>,
    mut scratch: Local<Option<LightingSnapshots<T>>>,
    mut engine: Local<super::propagation::SnapshotLightEngine>,
) {
    let context = scratch.get_or_insert_with(|| {
        LightingSnapshots::new(layout.min_y(), layout.height(), content.0.clone())
    });
    context.prepare(layout.min_y(), layout.height(), content.0.clone());
    for &position in &edits.0 {
        let center = layout.split(position).0;
        for x in -1..=1 {
            for z in -1..=1 {
                let position = center + IVec2::new(x, z);
                if !relevant.insert(position) {
                    continue;
                }
                if let Some(&entity) = index.get(&position)
                    && let Ok(voxels) = chunks.get(entity)
                {
                    context.insert(LightingChunk {
                        x: position.x,
                        z: position.y,
                        states: voxels.states.clone(),
                        light: voxels.render_light().clone(),
                        modified: false,
                        changed_cells: None,
                    });
                }
            }
        }
    }
    relevant.clear();
    context.block_edits(edits.0.drain(), &mut engine);
    for (x, z, light, sections) in context.drain() {
        let position = IVec2::new(x, z);
        if let Some(&entity) = index.get(&position)
            && let Ok(mut voxels) = chunks.get_mut(entity)
        {
            voxels.bypass_change_detection().estimate_block_light(light);
            for (slot, sections) in sections
                .into_iter()
                .enumerate()
                .filter(|(_, mask)| *mask != 0)
            {
                let offset = IVec2::new((slot % 3) as i32 - 1, (slot / 3) as i32 - 1);
                let (mask, own) = changed.entry(position + offset).or_default();
                *mask |= sections;
                *own |= slot == 4;
            }
        }
    }
    context.release_excess();
    engine.release_excess();
    for (position, (sections, own)) in changed.drain() {
        if let Some(&entity) = index.get(&position)
            && let Ok(mut voxels) = chunks.get_mut(entity)
        {
            voxels.mark_sections(sections);
            commands
                .entity(entity)
                .try_insert(crate::edits::EditedChunk);
            if own {
                commands.trigger(super::ChunkLightChanged(entity));
            }
        }
    }
}

pub fn added_lighting(_event: On<Add<VoxelChunk>>, mut version: ResMut<LightingVersion>) {
    version.0 = version.0.wrapping_add(1);
}

pub fn removed_lighting(_event: On<Remove<VoxelChunk>>, mut version: ResMut<LightingVersion>) {
    version.0 = version.0.wrapping_add(1);
}

struct Output<T: VoxelCell> {
    context: LightingSnapshots<T>,
    updates: Vec<(IVec2, Layer<u8>)>,
}

struct LightEdits {
    count: u64,
    cells: Option<Vec<u32>>,
}

#[derive(Resource)]
pub struct LightingState<T: VoxelCell> {
    context: Option<LightingSnapshots<T>>,
    task: Option<Task<Output<T>>>,
    submitted: HashMap<IVec2, Version>,
    ready: HashMap<IVec2, Layer<u8>>,
    synchronized_version: Option<u64>,
    changes: HashMap<IVec2, LightEdits>,
}

impl<T: VoxelCell> LightingState<T> {
    /// Light outside the edit neighborhoods submitted to the worker is already authoritative.
    pub fn is_settled(&self, position: IVec2, version: &LightingVersion) -> bool {
        !self.submitted.contains_key(&position)
            || (self.task.is_none()
                && self.ready.is_empty()
                && self.synchronized_version == Some(version.0))
    }
}

pub fn update_lighting<T: VoxelCell>(
    mut commands: Commands,
    mut state: ResMut<LightingState<T>>,
    version: Res<LightingVersion>,
    content: Res<crate::content::RegisteredContent>,
    layout: Res<WorldLayout>,
    budget: Res<crate::tasks::VoxelTaskBudget>,
    index: Res<ChunkIndex>,
    edited: Query<&VoxelChunk, With<LitEdits>>,
    mut chunks: Query<(Entity, &mut ChunkVoxels<T>)>,
    mut relevant: Local<HashSet<IVec2>>,
    mut edited_positions: Local<Vec<IVec2>>,
    mut current_positions: Local<Vec<IVec2>>,
) {
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
    state
        .changes
        .retain(|position, _| relevant.contains(position) && index.contains_key(position));
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
        // A partial/missed event stream must fall back to comparing shared sections.
        let changed_cells = state
            .changes
            .remove(position)
            .and_then(|journal| {
                let previous = state.submitted.get(position)?;
                (previous.0 == entity
                    && voxels.content_revision.checked_sub(previous.1) == Some(journal.count))
                .then_some(journal.cells)
                .flatten()
            })
            .map(|mut cells| {
                cells.sort_unstable();
                cells.dedup();
                cells
            });
        snapshots.push(LightingChunk {
            x: position.x,
            z: position.y,
            states: states.clone(),
            light: voxels.light.clone(),
            modified: voxels.modified,
            changed_cells,
        });
        state.submitted.insert(*position, version);
    }
    // Keep only results near unsynchronized edits pending. Unrelated edits and
    // streaming must not postpone light that is already valid for display.
    let invalidated: HashSet<_> = snapshots
        .iter()
        .map(|chunk| IVec2::new(chunk.x, chunk.z))
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
                commands.trigger(super::ChunkLightChanged(entity));
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
        let content = content.0.clone();
        let layout = layout.0;
        let removed = removed.into_iter().map(|p| (p.x, p.y)).collect::<Vec<_>>();
        state.task = Some(
            budget.spawn(crate::tasks::WorkClass::Lighting, 0, async move {
                let mut context = context.unwrap_or_else(|| {
                    LightingSnapshots::new(layout.min_y(), layout.height(), content)
                });
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

impl<T: VoxelCell> Default for LightingState<T> {
    fn default() -> Self {
        Self {
            context: None,
            task: None,
            submitted: HashMap::default(),
            ready: HashMap::default(),
            synchronized_version: None,
            changes: HashMap::default(),
        }
    }
}
