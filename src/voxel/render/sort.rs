//! Translucent sections are sorted back to front when meshed, then re-sorted as
//! the eye moves: from voxel to voxel near the eye, from chunk to chunk farther
//! away, where the order of a section's quads changes far more slowly. Their
//! meshes keep a main-world copy so new indices never need a remesh.
use super::model_mesh::SortData;
use crate::{
    game::GameState,
    player::PlayerCamera,
    voxel::coordinates::{CHUNK_SIZE, SECTION_HEIGHT},
};
use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

/// Sections within this many blocks of the eye (per axis, outside the section's
/// box) re-sort per voxel: two chunks horizontally, one section vertically.
const NEAR_HORIZONTAL: f32 = 2.0 * CHUNK_SIZE as f32;
const NEAR_VERTICAL: f32 = SECTION_HEIGHT as f32;
/// Concurrent re-sorts: each one re-uploads a whole translucent mesh.
const MAX_SORTS: usize = 2;

/// Eye cell a section's indices are sorted for: a voxel near the section, a
/// chunk-sized cell farther away. The flag keeps both grids apart.
pub(super) type SortKey = (IVec3, bool);

/// Sort key of a world-space eye for a section, given the section-local eye.
pub(super) fn sort_key(eye: Vec3, local_eye: Vec3) -> SortKey {
    let size = Vec3::new(CHUNK_SIZE as f32, SECTION_HEIGHT as f32, CHUNK_SIZE as f32);
    let outside = (-local_eye).max(local_eye - size).max(Vec3::ZERO);
    let near = outside.x.max(outside.z) <= NEAR_HORIZONTAL && outside.y <= NEAR_VERTICAL;
    if near {
        (eye.floor().as_ivec3(), true)
    } else {
        ((eye / CHUNK_SIZE as f32).floor().as_ivec3(), false)
    }
}

struct Output {
    indices: bevy::mesh::Indices,
    key: SortKey,
    mesh: AssetId<Mesh>,
}

#[derive(Component)]
pub(super) struct SortState {
    /// Eye cell the current indices were sorted for; `None` while unsorted.
    key: Option<SortKey>,
    task: Option<Task<Output>>,
}

impl SortState {
    pub(super) fn new(key: Option<SortKey>) -> Self {
        Self { key, task: None }
    }
}

pub(super) fn register(app: &mut App) {
    app.add_systems(
        PostUpdate,
        sort_translucent
            .after(bevy::transform::TransformSystems::Propagate)
            .run_if(in_state(GameState::Game)),
    );
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
    mut candidates: Local<Vec<(f32, Entity, SortKey, Vec3)>>,
) {
    let Some(camera) = camera else {
        return;
    };
    let eye = camera.translation();
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
                && output.key == key
                && let Some(mut target) = meshes.get_mut(&mesh.0)
            {
                target.insert_indices(output.indices);
                state.key = Some(key);
                applied += 1;
            }
        }
        if state.task.is_some() {
            active += 1;
            continue;
        }
        if visible.get() && state.key != Some(key) {
            candidates.push((
                (origin + Vec3::splat(8.0)).distance_squared(eye),
                entity,
                key,
                local_eye,
            ));
        }
    }
    let available = MAX_SORTS.saturating_sub(active);
    if candidates.len() > available {
        candidates.select_nth_unstable_by(available, |a, b| a.0.total_cmp(&b.0));
        candidates.truncate(available);
    }
    for &(_, entity, key, local_eye) in candidates.iter() {
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
