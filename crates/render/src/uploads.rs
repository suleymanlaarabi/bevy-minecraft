//! Keep visible geometry until its replacement has reached the render world.

use bevy::{
    platform::collections::HashSet,
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        mesh::RenderMesh,
        render_asset::{RenderAssets, prepare_assets},
    },
};
use std::sync::{Arc, Mutex};

use super::{
    model_mesh::SortData,
    sort::{SortKey, SortState, SortVersion},
};

/// Replacement geometry of an existing section entity. Sorting data must switch
/// together with the matching vertex/index buffers. Transient, so stored sparsely:
/// adding and removing it never moves the entity's table row.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct PendingMesh {
    pub mesh: Handle<Mesh>,
    pub sort: Option<(SortData, Option<SortKey>)>,
}

#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
struct MeshUploads {
    pending: Vec<AssetId<Mesh>>,
    ready: Arc<Mutex<HashSet<AssetId<Mesh>>>>,
}

pub fn register(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<MeshUploads>::default())
        .init_resource::<MeshUploads>()
        .add_systems(
            PostUpdate,
            commit_meshes
                .after(crate::VoxelRenderSystems::Build)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.add_systems(
            Render,
            report_ready_meshes
                .in_set(RenderSystems::PrepareAssets)
                .after(prepare_assets::<RenderMesh>),
        );
    }
}

fn report_ready_meshes(uploads: Res<MeshUploads>, meshes: Res<RenderAssets<RenderMesh>>) {
    let mut ready = uploads.ready.lock().unwrap();
    ready.clear();
    ready.extend(
        uploads
            .pending
            .iter()
            .copied()
            .filter(|id| meshes.get(*id).is_some()),
    );
}

fn commit_meshes(
    mut commands: Commands,
    mut uploads: ResMut<MeshUploads>,
    mut version: ResMut<SortVersion>,
    mut meshes: Query<(
        Entity,
        &mut Mesh3d,
        &mut PendingMesh,
        Option<(&mut SortData, &mut SortState)>,
    )>,
) {
    // An untouched resource is not extracted again.
    if meshes.is_empty() && uploads.pending.is_empty() {
        return;
    }
    let ready = uploads.ready.clone();
    let ready = ready.lock().unwrap();
    uploads.pending.clear();
    for (entity, mut current, mut pending, sort) in &mut meshes {
        let id = pending.mesh.id();
        if !ready.contains(&id) {
            uploads.pending.push(id);
            continue;
        }
        current.set_if_neq(Mesh3d(pending.mesh.clone()));
        if let Some((data, key)) = pending.sort.take() {
            if let Some((mut current_data, mut state)) = sort {
                // Replaced in place: no archetype move, and a fresh state drops
                // any sort of the previous mesh.
                *current_data = data;
                *state = SortState::new(key);
                version.0 = version.0.wrapping_add(1);
            } else {
                commands.entity(entity).insert((data, SortState::new(key)));
            }
        }
        commands.entity(entity).remove::<PendingMesh>();
    }
}
