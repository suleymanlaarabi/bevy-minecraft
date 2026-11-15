//! Shared chest geometry, bounded entity queues, and relationship-driven views.
use super::{
    catalog::{profile, state_behavior},
    containers::{
        BlockEntities, BlockEntity, BlockOf, BlockState, Dormant, LoadedTerrain, PairedWith,
    },
};
use crate::inventory::StationViewers;
use crate::voxel::{VoxelState, VoxelWorld, render::models};
use bevy::ecs::entity::EntityHashSet;
use bevy::{mesh::MeshTag, prelude::*};
use bevy_voxel::features::articulation::{
    Easing, JointAnimation, JointClip, JointMotion, SetJointTarget,
};
use bevy_voxel::render::{material::ModelMaterial, model_mesh::ModelGeometry, models::ModelQuad};

use bevy_voxel::render::instances::{self, InstanceOf, InstanceView, Instances, instance_light};
impl InstanceView for ChestView {
    fn meshes(&self, root: Entity) -> impl IntoIterator<Item = Entity> {
        [root, self.lid]
    }
}

const VIEW_BUDGET: usize = 64;
#[derive(Resource, Default)]
pub(crate) struct ChestMeshes {
    meshes: Option<[[Handle<Mesh>; 2]; 3]>,
    pivots: [Vec3; 3],
    material: Handle<ModelMaterial>,
    opening: Handle<JointClip>,
}
#[derive(Component)]
struct ChestView {
    lid: Entity,
    state: VoxelState,
}
#[derive(Component)]
#[component(storage = "SparseSet", on_add = bevy_voxel::engine::work::enqueue::<Self>)]
pub(crate) struct PrepareView;
type ViewQueue = bevy_voxel::engine::work::EntityWork<PrepareView>;
pub(crate) fn register(app: &mut App) {
    instances::register::<VoxelState>(app);
    app.init_resource::<ViewQueue>()
        .add_systems(
            OnExit(crate::game::GameState::Game),
            clear_updates.after(crate::voxel::streaming::reset_voxel_world),
        )
        .add_observer(render_requested)
        .add_systems(
            PostUpdate,
            (
                prepare_views,
                instances::update_lighting::<VoxelState, ChestView, 256>.after(prepare_views),
                update_open_lids
                    .after(prepare_views)
                    .before(bevy_voxel::features::articulation::ArticulationSystems),
            )
                .in_set(super::GameplaySystems::Presentation)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(crate::world::streaming),
        );
}
fn box_geometry(lo: Vec3, hi: Vec3, offset: Vec3) -> ModelGeometry {
    let mut result = ModelGeometry::default();
    let kind = crate::voxel::VoxelKind::from_key("chest").expect("chest registry");
    for face in (bevy_voxel::core::shape::BlockBox { min: lo, max: hi }).faces() {
        result.quad(
            &ModelQuad::face(face, models::fallback_slot(kind, face.axis, face.positive)),
            offset,
            [60; 4],
            0,
            None,
        );
    }
    result
}
impl ChestMeshes {
    fn prepare(
        &mut self,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<ModelMaterial>,
        assets: &crate::voxel::VoxelAssets,
        clips: &mut Assets<JointClip>,
    ) {
        if self.meshes.is_some() {
            return;
        }
        let mut material = materials
            .get(&assets.models_cutout)
            .expect("shared model material")
            .clone();
        material.instance_light = true;
        self.material = materials.add(material);
        self.opening = clips.add(JointClip {
            motions: Box::new([JointMotion::Rotation {
                axis: Dir3::X,
                radians: -std::f32::consts::FRAC_PI_2,
            }]),
            duration_seconds: 0.25,
            easing: Easing::CubicOut,
        });
        self.meshes = Some(core::array::from_fn(|half| {
            if let Some(parts) = models::chest_parts(half) {
                self.pivots[half] = parts.pivot;
                [&parts.body, &parts.lid].map(|quads| {
                    let mut geometry = ModelGeometry::default();
                    for q in quads.iter() {
                        geometry.quad(q, Vec3::ZERO, [60; 4], 0, None);
                    }
                    meshes.add(geometry.into_mesh())
                })
            } else {
                let pivot = Vec3::new(0.5, 0.625, 0.9375);
                self.pivots[half] = pivot;
                let lo = Vec3::new(if half == 2 { 0.0 } else { 0.0625 }, 0.0, 0.0625);
                let hi = Vec3::new(if half == 1 { 1.0 } else { 0.9375 }, 0.875, 0.9375);
                [
                    meshes.add(box_geometry(lo, hi.with_y(0.625), Vec3::ZERO).into_mesh()),
                    meshes.add(box_geometry(lo.with_y(0.625), hi, -pivot).into_mesh()),
                ]
            }
        }));
    }
}
fn prepare_views(
    mut commands: Commands,
    mut queue: ResMut<ViewQueue>,
    blocks: Query<
        (&BlockEntity, &BlockOf, &BlockState, Option<&Instances>),
        (With<PrepareView>, Without<Dormant>),
    >,
    columns: Query<&LoadedTerrain>,
    rendered: Query<(), With<bevy_voxel::render::streaming::RenderRequested>>,
    views: Query<&ChestView>,
    world: VoxelWorld,
    mut cache: ResMut<ChestMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ModelMaterial>>,
    assets: Res<crate::voxel::VoxelAssets>,
    mut clips: ResMut<Assets<JointClip>>,
) {
    for _ in 0..VIEW_BUDGET {
        let Some(entity) = queue.pop_front() else {
            break;
        };
        commands.entity(entity).try_remove::<PrepareView>();
        let Ok((block, owner, state, existing)) = blocks.get(entity) else {
            continue;
        };
        if !profile(state.0.kind()).chest {
            continue;
        }
        let Some(&terrain) = columns
            .get(owner.0)
            .ok()
            .and_then(|terrain| terrain.first())
        else {
            continue;
        };
        if !rendered.contains(terrain) {
            continue;
        }
        if existing.is_some_and(|existing| {
            existing
                .iter()
                .any(|view| views.get(view).is_ok_and(|view| view.state == state.0))
        }) {
            continue;
        }
        commands.entity(entity).despawn_related::<Instances>();
        cache.prepare(&mut meshes, &mut materials, &assets, &mut clips);
        let info = state_behavior(state.0);
        let half = info.chest_half as usize;
        let meshes = &cache.meshes.as_ref().expect("Prepared meshes")[half];
        // Imported canonical chests face south.
        let rotation =
            Quat::from_rotation_y((2.0 - info.facing as f32) * std::f32::consts::FRAC_PI_2);
        let center = Vec3::new(0.5, 0.0, 0.5);
        let local = crate::voxel::split_world_position(block.position)
            .1
            .as_vec3();
        let tag = MeshTag::new(instance_light(&world, block.position));
        let root = commands
            .spawn((
                InstanceOf(entity),
                ChildOf(terrain),
                Mesh3d(meshes[0].clone()),
                MeshMaterial3d(cache.material.clone()),
                Transform::from_translation(local + center - rotation * center)
                    .with_rotation(rotation),
                Visibility::default(),
                tag.clone(),
            ))
            .id();
        let lid = commands
            .spawn((
                Mesh3d(meshes[1].clone()),
                MeshMaterial3d(cache.material.clone()),
                Transform::from_translation(cache.pivots[half]),
                Visibility::default(),
                ChildOf(root),
                tag,
                JointAnimation::new(cache.opening.clone()),
            ))
            .id();
        commands.entity(root).insert(ChestView {
            lid,
            state: state.0,
        });
    }
}
fn update_open_lids(
    mut commands: Commands,
    blocks: Query<
        (
            Entity,
            &BlockEntity,
            Ref<Instances>,
            Option<Ref<PairedWith>>,
        ),
        Without<Dormant>,
    >,
    owners: Query<Ref<StationViewers>>,
    mut removed_users: RemovedComponents<StationViewers>,
    mut removed_pairs: RemovedComponents<PairedWith>,
    mut removed: Local<EntityHashSet>,
    views: Query<&ChestView>,
    lids: Query<&JointAnimation>,
) {
    // Removal batches contain sparse generational entity IDs; reuse their small set.
    removed.clear();
    removed.extend(removed_users.read().chain(removed_pairs.read()));
    for (entity, block, block_views, pair) in &blocks {
        let canonical = pair.as_ref().map_or(entity, |pair| pair.0);
        let users = owners.get(canonical).ok();
        let changed = block_views.is_changed()
            || pair.as_ref().is_some_and(|pair| pair.is_changed())
            || users.as_ref().is_some_and(|users| users.is_changed())
            || removed.contains(&canonical)
            || removed.contains(&entity);
        if !changed {
            continue;
        }
        let target = if users.is_some_and(|users| !users.is_empty()) {
            1.0
        } else {
            0.0
        };
        for root in block_views.iter() {
            if let Ok(view) = views.get(root)
                && let Ok(lid) = lids.get(view.lid)
                && lid.target != target
            {
                if canonical == entity {
                    commands.trigger(crate::audio::Sound::at(
                        if target > 0.0 {
                            crate::audio::SoundId::BlockChestOpen
                        } else {
                            crate::audio::SoundId::BlockChestClose
                        },
                        block.position.as_vec3() + Vec3::splat(0.5),
                    ));
                }
                commands.trigger(SetJointTarget {
                    entity: view.lid,
                    target,
                });
            }
        }
    }
}
fn clear_updates(mut views: ResMut<ViewQueue>) {
    views.clear();
}

fn render_requested(
    event: On<Add<bevy_voxel::render::streaming::RenderRequested>>,
    terrain: Query<&super::containers::LoadedColumn>,
    blocks: Query<&BlockEntities>,
    chests: Query<(), With<super::containers::Chest>>,
    mut commands: Commands,
) {
    let Ok(owner) = terrain.get(event.entity) else {
        return;
    };
    if let Ok(blocks) = blocks.get(owner.0) {
        for entity in blocks.iter() {
            if chests.contains(entity) {
                commands.entity(entity).insert(PrepareView);
            }
        }
    }
}
