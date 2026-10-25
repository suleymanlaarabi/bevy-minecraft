//! Translucent sections are sorted back to front when meshed, then re-sorted as
//! the eye moves. Grid-cell ordering stays valid when the camera turns. Meshes
//! keep a main-world copy so new indices never need a remesh.
use super::model_mesh::SortData;
use crate::{
    game::GameState,
    player::PlayerCamera,
    voxel::coordinates::{CHUNK_SIZE, SECTION_HEIGHT},
};
use bevy::{
    core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d},
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        render_phase::{ViewSortedRenderPhases, sort_phase_system},
        sync_world::MainEntityHashMap,
        view::ExtractedView,
    },
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

/// Sections within this many blocks of the eye (per axis, outside the section's
/// box) track quarter-block movement: two chunks horizontally, one section vertically.
const NEAR_HORIZONTAL: f32 = 2.0 * CHUNK_SIZE as f32;
const NEAR_VERTICAL: f32 = SECTION_HEIGHT as f32;
const NEAR_POSITION_STEPS: f32 = 4.0;
/// Farther away the order of a section's cells changes far more slowly: re-sort
/// once per chunk-sized eye cell.
const FAR_POSITION_STEPS: f32 = 1.0 / CHUNK_SIZE as f32;
/// Concurrent re-sorts and applications per frame; a changed order uploads the mesh.
const MAX_SORTS: usize = 2;
const SECTION_SIZE: Vec3 = Vec3::new(CHUNK_SIZE as f32, SECTION_HEIGHT as f32, CHUNK_SIZE as f32);

/// Quantized position and position-grid discriminator.
pub(super) type SortKey = (IVec3, bool);

/// Sort key of a world-space eye for a section, given the section-local eye.
pub(super) fn sort_key(eye: Vec3, local_eye: Vec3) -> SortKey {
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
    indices: bevy::mesh::Indices,
    key: SortKey,
    mesh: AssetId<Mesh>,
}

#[derive(Component)]
pub(super) struct SortState {
    /// View key the current indices were sorted for; `None` while unsorted.
    key: Option<SortKey>,
    task: Option<Task<Output>>,
    /// Frame of the last applied sort: the slots after the nearest one go to the
    /// stalest sections, so none starves during continuous movement.
    last_sorted: u64,
}

impl SortState {
    pub(super) fn new(key: Option<SortKey>) -> Self {
        Self {
            key,
            task: None,
            last_sorted: 0,
        }
    }
}

pub(super) fn register(app: &mut App) {
    app.add_systems(
        PostUpdate,
        sort_translucent
            .after(bevy::transform::TransformSystems::Propagate)
            .run_if(in_state(GameState::Game)),
    );
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .init_resource::<SectionCenters>()
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
    sections: Extract<Query<(Entity, &GlobalTransform), With<SortState>>>,
) {
    centers.0.clear();
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
    camera: Option<Single<&GlobalTransform, With<PlayerCamera>>>,
    mut sections: Query<(
        Entity,
        &Mesh3d,
        &GlobalTransform,
        &ViewVisibility,
        &SortData,
        &mut SortState,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut candidates: Local<Vec<(u64, f32, Entity, SortKey, Vec3)>>,
    mut frame: Local<u64>,
) {
    let Some(camera) = camera else {
        return;
    };
    let eye = camera.translation();
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
            if output.mesh == mesh.0.id()
                && let Some(target) = meshes.get(&mesh.0)
            {
                // Movement must not starve uploads by invalidating every result.
                // Record its captured view, then queue a refresh below if needed.
                // Avoid re-uploading vertices when the index order did not change.
                if target.indices() != Some(&output.indices)
                    && let Some(mut target) = meshes.get_mut(&mesh.0)
                {
                    target.insert_indices(output.indices);
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
    for &(_, _, entity, key, local_eye) in candidates.iter() {
        let Ok((_, mesh, _, _, sort, mut state)) = sections.get_mut(entity) else {
            continue;
        };
        let sort = sort.clone();
        let mesh = mesh.0.id();
        state.task = Some(AsyncComputeTaskPool::get().spawn(async move {
            Output {
                indices: sort.mesh_indices(local_eye),
                key,
                mesh,
            }
        }));
    }
}
