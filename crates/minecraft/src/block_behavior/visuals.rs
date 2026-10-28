//! Shared chest geometry, bounded entity queues, and relationship-driven views.
use super::{
    catalog::{profile, state_behavior},
    containers::{
        BlockEntities, BlockEntity, BlockOf, BlockState, Dormant, GameplayColumns, LoadedTerrain,
        PairedWith,
    },
};
use crate::inventory::StationViewers;
use crate::voxel::{VoxelState, VoxelWorld, render::models};
use bevy::ecs::{entity::EntityHashSet, system::SystemParam};
use bevy::{mesh::MeshTag, prelude::*};
use hollow_features::articulation::{
    Easing, JointAnimation, JointClip, JointMotion, SetJointTarget,
};
use hollow_render::{material::ModelMaterial, model_mesh::ModelGeometry, models::ModelQuad};

const VIEW_BUDGET: usize = 64;
const LIGHT_BUDGET: usize = 256;
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
#[relationship(relationship_target = BlockViews)]
struct ViewOf(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = ViewOf, linked_spawn)]
struct BlockViews(Vec<Entity>);
#[derive(Component)]
#[component(storage = "SparseSet", on_add = hollow_engine::work::enqueue::<Self>)]
pub(crate) struct PrepareView;
#[derive(Component, Default)]
#[component(storage = "SparseSet", on_add = hollow_engine::work::enqueue::<Self>)]
struct Relight {
    offset: usize,
    rerun: bool,
}
type ViewQueue = hollow_engine::work::EntityWork<PrepareView>;
type LightQueue = hollow_engine::work::EntityWork<Relight>;
#[derive(Component, Default)]
pub(crate) struct InstanceLight(u32);

#[derive(SystemParam)]
pub(crate) struct LightInvalidation<'w, 's> {
    columns: Res<'w, GameplayColumns>,
    progress: Query<'w, 's, &'static mut Relight>,
    commands: Commands<'w, 's>,
}
impl LightInvalidation<'_, '_> {
    fn column(&mut self, position: IVec2) {
        let Some(&entity) = self.columns.get(&position) else {
            return;
        };
        if let Ok(mut progress) = self.progress.get_mut(entity) {
            if !progress.rerun {
                progress.rerun = true;
            }
        } else {
            // Repeated invalidations before a flush must not replace active progress.
            self.commands
                .entity(entity)
                .try_insert_if_new(Relight::default());
        }
    }
    pub(crate) fn neighbors(&mut self, position: IVec2) {
        for offset in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            self.column(position + offset);
        }
    }
}
pub(crate) fn register(app: &mut App) {
    app.init_resource::<ViewQueue>()
        .init_resource::<LightQueue>()
        .add_systems(
            OnExit(crate::game::GameState::Game),
            clear_updates.after(crate::voxel::streaming::reset_voxel_world),
        )
        .add_observer(render_requested)
        .add_observer(changed)
        .add_observer(light_changed)
        .add_observer(chunk_departed)
        .add_systems(
            PostUpdate,
            (
                prepare_views,
                update_lighting.after(prepare_views),
                update_open_lids
                    .after(prepare_views)
                    .before(hollow_features::articulation::ArticulationSystems),
            )
                .in_set(super::GameplaySystems::Presentation)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(crate::world::streaming),
        );
}
fn changed(event: On<crate::voxel::edits::BlockChanged>, mut invalidation: LightInvalidation) {
    let positions = [IVec3::ZERO, IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z]
        .map(|delta| crate::voxel::split_world_position(event.position + delta).0);
    for (index, &position) in positions.iter().enumerate() {
        if !positions[..index].contains(&position) {
            invalidation.column(position);
        }
    }
}
fn chunk_departed(
    event: On<Despawn<crate::voxel::VoxelChunk>>,
    chunks: Query<&crate::voxel::VoxelChunk>,
    mut invalidation: LightInvalidation,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        invalidation.neighbors(chunk.position);
    }
}
fn light_changed(
    event: On<hollow_engine::lighting::ChunkLightChanged>,
    chunks: Query<&crate::voxel::VoxelChunk>,
    mut invalidation: LightInvalidation,
) {
    if let Ok(chunk) = chunks.get(event.0) {
        invalidation.neighbors(chunk.position);
    }
}
fn box_geometry(lo: Vec3, hi: Vec3, offset: Vec3) -> ModelGeometry {
    let mut result = ModelGeometry::default();
    let kind = crate::voxel::VoxelKind::from_key("chest").expect("chest registry");
    for face in (hollow_core::shape::BlockBox { min: lo, max: hi }).faces() {
        result.quad(
            &ModelQuad {
                positions: face.positions.map(|position| position.to_array()),
                normal: face.normal.to_array(),
                uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                texture: models::fallback_slot(kind, face.axis, face.positive),
                tinted: false,
                cull: [0; 3],
                translucent: false,
                effect: 0,
                light_emission: 0,
                shade_up: false,
            },
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
fn light(world: &VoxelWorld, position: IVec3) -> u32 {
    let mut sky = 0;
    let mut block = 0;
    for delta in [
        IVec3::ZERO,
        IVec3::Y,
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Z,
        IVec3::NEG_Z,
    ] {
        if let Some(value) = world.light(position + delta) {
            sky = sky.max(value >> 4);
            block = block.max(value & 15);
        }
    }
    u32::from((sky << 4) | block)
}
fn prepare_views(
    mut commands: Commands,
    mut queue: ResMut<ViewQueue>,
    blocks: Query<
        (&BlockEntity, &BlockOf, &BlockState, Option<&BlockViews>),
        (With<PrepareView>, Without<Dormant>),
    >,
    columns: Query<&LoadedTerrain>,
    rendered: Query<(), With<hollow_render::streaming::RenderRequested>>,
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
        commands.entity(entity).despawn_related::<BlockViews>();
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
        let tag = MeshTag::new(light(&world, block.position));
        let root = commands
            .spawn((
                ViewOf(entity),
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
fn update_lighting(
    mut commands: Commands,
    mut queue: ResMut<LightQueue>,
    mut samples: Local<EntityHashSet>,
    mut columns: Query<(Option<&BlockEntities>, &mut Relight)>,
    mut blocks: Query<(&BlockEntity, &BlockViews, &mut InstanceLight), Without<Dormant>>,
    views: Query<&ChestView>,
    mut tags: Query<&mut MeshTag>,
    world: VoxelWorld,
) {
    samples.clear();
    for _ in 0..LIGHT_BUDGET {
        let Some(column) = queue.pop_front() else {
            break;
        };
        let Ok((blocks_in_column, mut progress)) = columns.get_mut(column) else {
            continue;
        };
        if let Some(entity) = blocks_in_column
            .and_then(|blocks| blocks.get(progress.offset))
            .copied()
        {
            samples.insert(entity);
        }
        progress.offset += 1;
        if progress.offset >= blocks_in_column.map_or(0, |blocks| blocks.len()) {
            if !progress.rerun {
                commands.entity(column).remove::<Relight>();
                continue;
            }
            progress.offset = 0;
            progress.rerun = false;
        }
        queue.push_back(column);
    }
    blocks
        .par_iter_many_unique_mut(&*samples)
        .for_each(|result| {
            if let Ok((block, _, mut sample)) = result {
                sample.0 = light(&world, block.position);
            }
        });
    for &entity in samples.iter() {
        let Ok((_, block_views, sample)) = blocks.get(entity) else {
            continue;
        };
        for root in block_views.iter() {
            if let Ok(view) = views.get(root) {
                for target in [root, view.lid] {
                    if let Ok(mut tag) = tags.get_mut(target) {
                        tag.set_if_neq(MeshTag::new(sample.0));
                    }
                }
            }
        }
    }
}
fn update_open_lids(
    mut commands: Commands,
    blocks: Query<
        (
            Entity,
            &BlockEntity,
            Ref<BlockViews>,
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
fn clear_updates(mut views: ResMut<ViewQueue>, mut lights: ResMut<LightQueue>) {
    views.clear();
    lights.clear();
}

fn render_requested(
    event: On<Add<hollow_render::streaming::RenderRequested>>,
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
