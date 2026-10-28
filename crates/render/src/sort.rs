//! Translucent sections are sorted back to front when meshed, then re-sorted as
//! the eye moves. Grid-cell ordering stays valid when the camera turns. Meshes
//! retain immutable GPU vertices; re-sorts patch only their index allocation.
use super::model_mesh::SortData;
use crate::VoxelViewer;
use bevy::mesh::Indices;
use bevy::platform::collections::HashMap;
use bevy::{
    core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d},
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        mesh::{RenderMesh, allocator::MeshAllocator},
        render_asset::RenderAssets,
        render_phase::{ViewSortedRenderPhases, sort_phase_system},
        render_resource::IndexFormat,
        renderer::RenderQueue,
        sync_world::{MainEntity, MainEntityHashMap},
        view::ExtractedView,
    },
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
const SECTION_SIZE: Vec3 = Vec3::new(CHUNK_SIZE as f32, SECTION_HEIGHT as f32, CHUNK_SIZE as f32);

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

struct Output {
    indices: Option<Arc<Indices>>,
    key: SortKey,
    mesh: AssetId<Mesh>,
}

#[derive(Component)]
pub struct SortState {
    /// View key the current indices were sorted for; `None` while unsorted.
    key: Option<SortKey>,
    task: Option<Task<Output>>,
    /// Frame of the last applied sort: the slots after the nearest one go to the
    /// stalest sections, so none starves during continuous movement.
    last_sorted: u64,
    indices: Option<Arc<Indices>>,
}

impl SortState {
    pub fn new(key: Option<SortKey>) -> Self {
        Self {
            key,
            task: None,
            last_sorted: 0,
            indices: None,
        }
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<IndexUpdates>()
        .init_resource::<SortVersion>()
        .add_observer(mark_sort_changed)
        .add_systems(
            PostUpdate,
            sort_translucent.after(
                bevy::camera::visibility::VisibilitySystems::MarkNewlyHiddenEntitiesInvisible,
            ),
        );
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .init_resource::<SectionCenters>()
            .init_resource::<PendingIndices>()
            .add_systems(ExtractSchedule, extract_index_updates)
            .add_systems(
                Render,
                upload_indices.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(ExtractSchedule, extract_section_centers)
            .add_systems(
                Render,
                prepare_section_sorting
                    .in_set(RenderSystems::PhaseSort)
                    .before(sort_phase_system::<Transparent3d>),
            );
    }
}

/// Canonical grid centers, independent of which faces happen to be exposed.
#[derive(Resource, Default)]
struct SectionCenters(MainEntityHashMap<Vec3>);

fn extract_section_centers(
    mut centers: ResMut<SectionCenters>,
    sections: Extract<
        Query<
            (Entity, &GlobalTransform),
            (
                With<SortState>,
                Or<(Added<SortState>, Changed<GlobalTransform>)>,
            ),
        >,
    >,
    mut removed: Extract<RemovedComponents<SortState>>,
) {
    for entity in removed.read() {
        centers.0.remove(&MainEntity::from(entity));
    }
    let half_size = SECTION_SIZE * 0.5;
    centers.0.extend(
        sections
            .iter()
            .map(|(entity, transform)| (entity.into(), transform.translation() + half_size)),
    );
}

fn prepare_section_sorting(
    centers: Res<SectionCenters>,
    views: Query<&ExtractedView>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
) {
    for view in &views {
        // The grid ordering is for perspective rays from a shared eye. Leave
        // orthographic views to Bevy's parallel-ray depth ordering.
        if view.clip_from_view.w_axis.w != 0.0 {
            continue;
        }
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let eye = view.world_from_view.translation();
        for item in phase.items.values_mut() {
            let Some(center) = centers.0.get(&item.entity.1) else {
                continue;
            };
            // View-depth sorting of partial section bounds can draw a whole
            // underwater section over nearer ice. Use the same radial grid
            // ordering as the block sort. A common sort point and radial bias
            // supply that key to Bevy's existing sorter without an extra sort.
            // Culling bounds and vertex positions retain their actual geometry.
            item.sorting_info = TransparentSortingInfo3d::Sorted {
                mesh_center: eye,
                depth_bias: -center.distance(eye),
            };
        }
    }
}

fn sort_translucent(
    budget: Res<hollow_engine::tasks::VoxelTaskBudget>,
    camera: Option<Single<&GlobalTransform, With<VoxelViewer>>>,
    mut sections: Query<(
        Entity,
        &Mesh3d,
        &GlobalTransform,
        &ViewVisibility,
        &SortData,
        &mut SortState,
    )>,
    visibility_changes: Query<(), (With<SortState>, Changed<ViewVisibility>)>,
    mut updates: ResMut<IndexUpdates>,
    version: Res<SortVersion>,
    mut previous: Local<Option<(Vec3, u64)>>,
    mut active_hint: Local<usize>,
    mut candidates: Local<Vec<(u64, f32, Entity, SortKey, Vec3)>>,
    mut frame: Local<u64>,
    #[cfg(feature = "dev")] diagnostics: Option<ResMut<crate::diagnostics::VoxelDiagnostics>>,
) {
    updates.0.clear();
    let Some(camera) = camera else {
        return;
    };
    let eye = camera.translation();
    let view_key = (eye, version.0);
    // Rotation cannot change a radial ordering. Bevy's visibility change ticks
    // identify newly exposed sections, including after projection changes.
    if *previous == Some(view_key) && visibility_changes.is_empty() && *active_hint == 0 {
        return;
    }
    #[cfg(feature = "dev")]
    let started = std::time::Instant::now();
    *previous = Some(view_key);
    *frame += 1;
    let mut active = 0;
    let mut applied = 0;
    candidates.clear();
    for (entity, mesh, transform, visible, _, mut state) in &mut sections {
        // Section transforms are pure translations.
        let origin = transform.translation();
        let local_eye = eye - origin;
        let key = sort_key(eye, local_eye);
        if let Some(task) = &mut state.task
            && applied < MAX_SORTS
            && let Some(output) = check_ready(task)
        {
            state.task = None;
            if output.mesh == mesh.0.id() {
                if let Some(indices) = output.indices {
                    updates.0.push((output.mesh, indices.clone()));
                    state.indices = Some(indices);
                }
                state.key = Some(output.key);
                state.last_sorted = *frame;
                applied += 1;
            }
        }
        if state.task.is_some() {
            active += 1;
            continue;
        }
        if visible.get() && state.key != Some(key) {
            candidates.push((
                state.last_sorted,
                (origin + SECTION_SIZE * 0.5).distance_squared(eye),
                entity,
                key,
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
    for &(_, _, entity, key, local_eye) in candidates.iter() {
        let Ok((_, mesh, _, _, sort, mut state)) = sections.get_mut(entity) else {
            continue;
        };
        let sort = sort.clone();
        let mesh = mesh.0.id();
        let previous = state.indices.clone();
        state.task = Some(budget.spawn(
            hollow_engine::tasks::WorkClass::Streaming,
            local_eye.length_squared() as u32,
            async move {
                let indices = sort.mesh_indices(local_eye);
                // Comparing and dropping a large unchanged payload belongs on the
                // worker too; the ECS thread only handles the shared result.
                let indices = (previous.as_deref() != Some(&indices)).then(|| Arc::new(indices));
                Output { indices, key, mesh }
            },
        ));
    }
    #[cfg(feature = "dev")]
    if let Some(mut diagnostics) = diagnostics {
        diagnostics.translucent_sort_seconds += started.elapsed().as_secs_f64();
    }
}

/// Only the latest immutable index payload crosses extraction; vertices stay untouched.
#[derive(Resource, Default)]
struct IndexUpdates(Vec<(AssetId<Mesh>, Arc<Indices>)>);

#[derive(Resource, Default)]
struct PendingIndices(HashMap<AssetId<Mesh>, Arc<Indices>>);

fn extract_index_updates(
    updates: Extract<Res<IndexUpdates>>,
    meshes: Extract<Res<Assets<Mesh>>>,
    mut pending: ResMut<PendingIndices>,
) {
    pending.0.extend(updates.0.iter().cloned());
    pending.0.retain(|mesh, _| meshes.contains(*mesh));
}

fn upload_indices(
    mut pending: ResMut<PendingIndices>,
    allocator: Res<MeshAllocator>,
    meshes: Res<RenderAssets<RenderMesh>>,
    queue: Res<RenderQueue>,
) {
    pending.0.retain(|id, indices| {
        let (Some(mesh), Some(slice)) = (meshes.get(*id), allocator.mesh_index_slice(id)) else {
            return true;
        };
        let (format, bytes): (IndexFormat, &[u8]) = match &**indices {
            Indices::U16(values) => (IndexFormat::Uint16, bytemuck::cast_slice(values)),
            Indices::U32(values) => (IndexFormat::Uint32, bytemuck::cast_slice(values)),
        };
        // The ID changes on remeshing. Never write across a replaced allocation.
        if mesh.index_format() == Some(format) && slice.range.len() >= indices.len() {
            let stride = match format {
                IndexFormat::Uint16 => 2,
                IndexFormat::Uint32 => 4,
            };
            queue.write_buffer(&slice.buffer, u64::from(slice.range.start) * stride, bytes);
        }
        false
    });
}

#[derive(Resource, Default)]
pub struct SortVersion(pub u64);

fn mark_sort_changed(_event: On<Insert<SortState>>, mut version: ResMut<SortVersion>) {
    version.0 = version.0.wrapping_add(1);
}
