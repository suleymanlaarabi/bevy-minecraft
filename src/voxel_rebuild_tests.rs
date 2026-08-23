use std::thread;

use super::*;
use bevy::{
    asset::RenderAssetUsages, render::render_resource::PrimitiveTopology, state::app::StatesPlugin,
};

#[test]
fn plugin_streams_a_generated_chunk_end_to_end() {
    use super::super::{VoxelChunk, VoxelPlugin, VoxelSettings, VoxelViewer};
    use crate::game::GameState;

    let settings = VoxelSettings {
        view_distance: 0,
        spawn_budget_per_frame: 1,
        ..default()
    };
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        StatesPlugin,
        TransformPlugin,
        VoxelPlugin::new(settings),
    ))
    .insert_state(GameState::Game)
    .init_resource::<Assets<Mesh>>()
    .init_resource::<Assets<StandardMaterial>>();
    app.finish();
    app.world_mut().spawn((VoxelViewer, Transform::default()));
    for _ in 0..2_000 {
        app.update();
        thread::yield_now();
        let ready = app
            .world_mut()
            .query_filtered::<Entity, (With<VoxelChunk>, With<Mesh3d>, With<Collider>)>()
            .iter(app.world())
            .next()
            .is_some();
        if ready {
            return;
        }
    }
    panic!("voxel plugin did not finish its first generated chunk");
}

#[test]
fn water_child_is_created_removed_and_despawned_with_its_chunk() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .insert_resource(VoxelAssets {
            terrain: default(),
            water: default(),
        })
        .add_systems(Update, poll_builds);
    app.finish();
    let parent = spawn_build(&mut app, true);
    let first_child = wait_for_water(&mut app, parent);
    assert_eq!(
        app.world().get::<ChildOf>(first_child).unwrap().parent(),
        parent
    );

    app.world_mut().entity_mut(parent).insert(build_task(false));
    for _ in 0..1_000 {
        app.update();
        thread::yield_now();
        if app.world().get::<ChunkWater>(parent).is_none() {
            break;
        }
    }
    assert!(!app.world().entities().contains(first_child));

    app.world_mut().entity_mut(parent).insert(build_task(true));
    let second_child = wait_for_water(&mut app, parent);
    app.world_mut().despawn(parent);
    assert!(!app.world().entities().contains(second_child));
}

fn spawn_build(app: &mut App, water: bool) -> Entity {
    app.world_mut().spawn(build_task(water)).id()
}

fn wait_for_water(app: &mut App, parent: Entity) -> Entity {
    for _ in 0..1_000 {
        app.update();
        thread::yield_now();
        if let Some(water) = app.world().get::<ChunkWater>(parent) {
            return water.entity;
        }
    }
    panic!("water child was not created");
}

fn build_task(water: bool) -> ChunkBuild {
    ChunkBuild {
        task: AsyncComputeTaskPool::get().spawn(async move {
            BuildOutput {
                meshes: ChunkMeshes(empty_mesh(), water.then(empty_mesh)),
                collider: ColliderUpdate::Keep,
            }
        }),
        flags: 0,
    }
}

fn empty_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD,
    )
}
