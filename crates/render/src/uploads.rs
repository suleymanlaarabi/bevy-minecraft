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

#[derive(Component)]
pub struct PendingMesh(pub Handle<Mesh>);

/// Sorting data must switch together with the matching vertex/index buffers.
#[derive(Component)]
pub struct PendingSort(pub Option<(super::model_mesh::SortData, Option<super::sort::SortKey>)>);

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
    mut meshes: Query<(Entity, &mut Mesh3d, &PendingMesh, Option<&mut PendingSort>)>,
) {
    let ready = uploads.ready.clone();
    let ready = ready.lock().unwrap();
    uploads.pending.clear();
    for (entity, mut current, pending, sort) in &mut meshes {
        if !ready.contains(&pending.0.id()) {
            uploads.pending.push(pending.0.id());
            continue;
        }
        current.set_if_neq(Mesh3d(pending.0.clone()));
        if let Some(mut sort) = sort
            && let Some((data, key)) = sort.0.take()
        {
            commands
                .entity(entity)
                .insert((data, super::sort::SortState::new(key)));
        }
        commands
            .entity(entity)
            .remove::<(PendingMesh, PendingSort)>();
    }
}
