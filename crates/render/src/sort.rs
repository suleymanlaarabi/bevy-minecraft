//! Translucent sections are sorted back to front when meshed, then re-sorted as
//! the eye moves. Grid-cell ordering stays valid when the camera turns. Section
//! arenas retain immutable vertices; re-sorts patch only their index allocation.
use super::model_mesh::SortData;
use crate::VoxelViewer;
use crate::sections::{SectionFeed, SectionOp};
use bevy::platform::collections::HashMap;
use bevy::{
    prelude::*,
    tasks::{Task, futures::check_ready},
};
use hollow_core::coordinates::{CHUNK_SIZE, SECTION_HEIGHT};
use std::sync::Arc;

/// Sections within this many blocks of the eye (per axis, outside the section's
/// box) track quarter-block movement: two chunks horizontally, one section vertically.
const NEAR_HORIZONTAL: f32 = 2.0 * CHUNK_SIZE as f32;
const NEAR_VERTICAL: f32 = SECTION_HEIGHT as f32;
const NEAR_POSITION_STEPS: f32 = 4.0;
/// Farther away the order of a section's cells changes far more slowly: re-sort
/// once per chunk-sized eye cell.
const FAR_POSITION_STEPS: f32 = 1.0 / CHUNK_SIZE as f32;
/// Concurrent re-sorts and applications per frame; a changed order uploads indices.
const MAX_SORTS: usize = 2;
pub(crate) const SECTION_SIZE: Vec3 =
    Vec3::new(CHUNK_SIZE as f32, SECTION_HEIGHT as f32, CHUNK_SIZE as f32);

/// Quantized position and position-grid discriminator.
pub type SortKey = (IVec3, bool);

/// Sort key of a world-space eye for a section, given the section-local eye.
pub fn sort_key(eye: Vec3, local_eye: Vec3) -> SortKey {
    let outside = (-local_eye).max(local_eye - SECTION_SIZE).max(Vec3::ZERO);
    let near = outside.x.max(outside.z) <= NEAR_HORIZONTAL && outside.y <= NEAR_VERTICAL;
    let position_steps = if near {
        NEAR_POSITION_STEPS
    } else {
        FAR_POSITION_STEPS
    };
    ((eye * position_steps).floor().as_ivec3(), near)
}

/// A column and section index.
type SectionKey = (IVec2, u8);
/// Indices that differ from the previous sort, and the key they were sorted for.
type SortOutput = (Option<Arc<[u32]>>, SortKey);

struct Section {
    origin: Vec3,
    sort: SortData,
    /// View key the current indices were sorted for; `None` while unsorted.
    key: Option<SortKey>,
    /// Dropping a replaced section cancels its sort.
    task: Option<Task<SortOutput>>,
    /// Frame of the last applied sort: the slots after the nearest one go to the
    /// stalest sections, so none starves during continuous movement.
    last_sorted: u64,
    indices: Option<Arc<[u32]>>,
}

/// Sort data of every translucent section in the section arenas, with the
/// visibility masks the camera culls them by.
#[derive(Resource, Default)]
pub struct TranslucentSections {
    sections: HashMap<SectionKey, Section>,
    /// Published visible sections per column; absent columns are shown.
    visible: HashMap<IVec2, u32>,
    conservative: bool,
    /// Changes whenever the set of sortable sections may have changed.
    version: u64,
}

impl TranslucentSections {
    pub(crate) fn replace(
        &mut self,
        column: IVec2,
        section: usize,
        origin: Vec3,
        sort: Option<(SortData, Option<SortKey>)>,
    ) {
        let key = (column, section as u8);
        match sort {
            Some((sort, key_sorted)) => {
                self.sections.insert(
                    key,
                    Section {
                        origin,
                        sort,
                        key: key_sorted,
                        task: None,
                        last_sorted: 0,
                        indices: None,
                    },
                );
            }
            None => {
                if self.sections.remove(&key).is_none() {
                    return;
                }
            }
        }
        self.version = self.version.wrapping_add(1);
    }
    pub(crate) fn remove_column(&mut self, column: IVec2) {
        self.sections.retain(|key, _| key.0 != column);
        self.visible.remove(&column);
        self.version = self.version.wrapping_add(1);
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
    /// Records a column's published mask; returns whether a translucent section
    /// changed visibility.
    pub(crate) fn publish(&mut self, column: IVec2, mask: u32, translucent: u32) {
        let previous = self.visible.insert(column, mask).unwrap_or(u32::MAX);
        if (previous ^ mask) & translucent != 0 {
            self.version = self.version.wrapping_add(1);
        }
    }
    pub(crate) fn set_conservative(&mut self, conservative: bool) {
        if self.conservative != conservative {
            self.conservative = conservative;
            self.version = self.version.wrapping_add(1);
        }
    }
    #[cfg(feature = "dev")]
    pub fn census(&self, census: &mut hollow_core::sections::AllocationCensus) {
        for section in self.sections.values() {
            section.sort.census(census);
        }
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<TranslucentSections>()
        .add_observer(
            |_: On<hollow_engine::session::SessionClosed>,
             mut sections: ResMut<TranslucentSections>| sections.clear(),
        )
        .add_systems(
            PostUpdate,
            sort_translucent.after(crate::VoxelRenderSystems::Build),
        );
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sort_translucent(
    budget: Res<hollow_engine::tasks::VoxelTaskBudget>,
    camera: Option<Single<&GlobalTransform, With<VoxelViewer>>>,
    mut translucent: ResMut<TranslucentSections>,
    mut feed: ResMut<SectionFeed>,
    mut previous: Local<Option<((IVec3, IVec3), u64)>>,
    mut active_hint: Local<usize>,
    mut candidates: Local<Vec<(u64, f32, SectionKey, SortKey, Vec3)>>,
    mut frame: Local<u64>,
    #[cfg(feature = "dev")] diagnostics: Option<ResMut<crate::diagnostics::VoxelDiagnostics>>,
) {
    let Some(camera) = camera else {
        return;
    };
    let eye = camera.translation();
    // Every section key is constant while the eye stays within one open
    // quarter-block cell or on one grid point: positions are quantized to
    // quarter or 16-block cells, and the near bounds lie on integer planes.
    let quarter = eye * NEAR_POSITION_STEPS;
    let view_key = (
        (quarter.floor().as_ivec3(), quarter.ceil().as_ivec3()),
        translucent.version,
    );
    // Rotation cannot change a radial ordering, and every visible section is kept
    // sorted whatever the camera faces.
    if *previous == Some(view_key) && *active_hint == 0 {
        return;
    }
    #[cfg(feature = "dev")]
    let started = std::time::Instant::now();
    *previous = Some(view_key);
    *frame += 1;
    let (mut active, mut applied) = (0, 0);
    candidates.clear();
    let TranslucentSections {
        sections,
        visible,
        conservative,
        ..
    } = &mut *translucent;
    for (&key, section) in sections.iter_mut() {
        if let Some(task) = section.task.as_mut().filter(|_| applied < MAX_SORTS)
            && let Some((indices, sorted)) = check_ready(task)
        {
            section.task = None;
            if let Some(indices) = indices {
                feed.ops.push(SectionOp::Sort {
                    column: key.0,
                    section: usize::from(key.1),
                    indices: indices.clone(),
                });
                section.indices = Some(indices);
            }
            section.key = Some(sorted);
            section.last_sorted = *frame;
            applied += 1;
        }
        if section.task.is_some() {
            active += 1;
            continue;
        }
        if !*conservative
            && visible
                .get(&key.0)
                .is_some_and(|mask| mask & 1 << key.1 == 0)
        {
            continue;
        }
        let local_eye = eye - section.origin;
        let sort = sort_key(eye, local_eye);
        if section.key != Some(sort) {
            candidates.push((
                section.last_sorted,
                (section.origin + SECTION_SIZE * 0.5).distance_squared(eye),
                key,
                sort,
                local_eye,
            ));
        }
    }
    let available = MAX_SORTS.saturating_sub(active);
    if candidates.len() > available && available > 0 {
        // The nearest section, whose order shows the most, always gets a slot.
        let nearest = (0..candidates.len())
            .min_by(|&a, &b| candidates[a].1.total_cmp(&candidates[b].1))
            .expect("more candidates than available slots");
        candidates.swap(0, nearest);
        let stalest = available - 1;
        if stalest > 0 {
            candidates[1..].select_nth_unstable_by(stalest, |a, b| {
                a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1))
            });
        }
    }
    candidates.truncate(available);
    *active_hint = active + candidates.len();
    for &(_, _, key, sorted, local_eye) in candidates.iter() {
        let Some(section) = sections.get_mut(&key) else {
            continue;
        };
        let sort = section.sort.clone();
        let previous = section.indices.clone();
        section.task = Some(budget.spawn(
            hollow_engine::tasks::WorkClass::Streaming,
            local_eye.length_squared() as u32,
            async move {
                let indices = sort.indices(local_eye);
                // Comparing and dropping a large unchanged payload belongs on the
                // worker too; the ECS thread only handles the shared result.
                let indices =
                    (previous.as_deref() != Some(indices.as_slice())).then(|| Arc::from(indices));
                (indices, sorted)
            },
        ));
    }
    #[cfg(feature = "dev")]
    if let Some(mut diagnostics) = diagnostics {
        diagnostics.translucent_sort_seconds += started.elapsed().as_secs_f64();
    }
}
