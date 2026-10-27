//! Shared chest geometry, bounded entity queues, and relationship-driven views.
use super::{
    catalog::{profile, state_behavior},
    containers::{
        BlockEntities, BlockEntity, BlockOf, BlockState, ContainerViewers, Dormant,
        GameplayColumns, LoadedTerrain, PairedWith,
    },
};
use crate::voxel::{
    VoxelState, VoxelWorld,
    render::{
        ModelMaterial,
        model_mesh::ModelGeometry,
        models::{self, ModelQuad},
    },
};
use bevy::ecs::entity::EntityHashSet;
use bevy::{ecs::system::ParallelCommands, mesh::MeshTag, prelude::*};
const VIEW_BUDGET: usize = 64;
const LIGHT_BUDGET: usize = 256;
#[derive(Resource, Default)]
pub(crate) struct ChestMeshes {
    meshes: Option<[[Handle<Mesh>; 2]; 3]>,
    pivots: [Vec3; 3],
    material: Handle<ModelMaterial>,
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
struct Lid {
    progress: f32,
    target: f32,
}
#[derive(Component)]
#[component(storage = "SparseSet")]
struct MovingLid;
#[derive(Component)]
#[component(storage = "SparseSet", on_add = super::queue::enqueue::<Self>)]
pub(crate) struct PrepareView;
#[derive(Component, Default)]
#[component(storage = "SparseSet", on_add = super::queue::enqueue::<Self>)]
struct Relight {
    offset: usize,
    rerun: bool,
}
type ViewQueue = super::queue::EntityWork<PrepareView>;
type LightQueue = super::queue::EntityWork<Relight>;
#[derive(Component, Default)]
pub(crate) struct InstanceLight(u32);
#[derive(Event)]
pub(crate) struct ChunkLightChanged(pub Entity);

pub(crate) fn invalidate_column(world: &mut World, column: Entity) {
    if let Some(mut progress) = world.get_mut::<Relight>(column) {
        progress.rerun = true;
    } else if let Ok(mut entity) = world.get_entity_mut(column) {
        entity.insert(Relight::default());
    }
}
pub(crate) fn invalidate_neighbors(world: &mut World, position: IVec2) {
    for offset in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
        if let Some(&entity) = world
            .resource::<GameplayColumns>()
            .get(&(position + offset))
        {
            invalidate_column(world, entity);
        }
    }
}
pub(crate) fn register(app: &mut App) {
    app.init_resource::<ViewQueue>()
        .init_resource::<LightQueue>()
        .add_systems(OnExit(crate::game::GameState::Game), clear_updates)
        .add_observer(changed)
        .add_observer(light_changed)
        .add_observer(chunk_departed)
        .add_systems(
            PostUpdate,
            (
                prepare_views,
                update_lighting.after(prepare_views),
                update_open_lids.after(prepare_views),
                animate_lids.after(update_open_lids),
            )
                .in_set(super::GameplaySystems::Presentation)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(crate::world::streaming),
        );
}
fn changed(event: On<crate::voxel::edits::BlockChanged>, mut commands: Commands) {
    let position = event.position;
    commands.queue(move |world: &mut World| {
        for delta in [IVec3::ZERO, IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
            let coordinate = crate::voxel::split_world_position(position + delta).0;
            if let Some(&owner) = world.resource::<GameplayColumns>().get(&coordinate) {
                invalidate_column(world, owner);
            }
        }
    });
}
fn chunk_departed(
    event: On<Despawn<crate::voxel::VoxelChunk>>,
    chunks: Query<&crate::voxel::VoxelChunk>,
    mut commands: Commands,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        let position = chunk.position;
        commands.queue(move |world: &mut World| invalidate_neighbors(world, position));
    }
}
fn light_changed(
    event: On<ChunkLightChanged>,
    chunks: Query<&crate::voxel::VoxelChunk>,
    mut commands: Commands,
) {
    if let Ok(chunk) = chunks.get(event.0) {
        let position = chunk.position;
        commands.queue(move |world: &mut World| invalidate_neighbors(world, position));
    }
}
fn box_geometry(lo: Vec3, hi: Vec3, offset: Vec3) -> ModelGeometry {
    let mut result = ModelGeometry::default();
    let kind = crate::voxel::VoxelKind::from_key("chest").expect("chest registry");
    for axis in 0..3 {
        for positive in [false, true] {
            let mut p = lo;
            p[axis] = if positive { hi[axis] } else { lo[axis] };
            let mut du = Vec3::ZERO;
            du[(axis + 1) % 3] = (hi - lo)[(axis + 1) % 3];
            let mut dv = Vec3::ZERO;
            dv[(axis + 2) % 3] = (hi - lo)[(axis + 2) % 3];
            let positions = if positive {
                [p, p + du, p + du + dv, p + dv]
            } else {
                [p, p + dv, p + du + dv, p + du]
            };
            let mut normal = Vec3::ZERO;
            normal[axis] = if positive { 1.0 } else { -1.0 };
            result.quad(
                &ModelQuad {
                    positions: positions.map(|p| p.to_array()),
                    normal: normal.to_array(),
                    uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                    texture: models::fallback_slot(kind, axis, positive),
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
    }
    result
}
impl ChestMeshes {
    fn prepare(
        &mut self,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<ModelMaterial>,
        assets: &crate::voxel::VoxelAssets,
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
    views: Query<&ChestView>,
    world: VoxelWorld,
    mut cache: ResMut<ChestMeshes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ModelMaterial>>,
    assets: Res<crate::voxel::VoxelAssets>,
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
        if existing.is_some_and(|existing| {
            existing
                .iter()
                .any(|view| views.get(view).is_ok_and(|view| view.state == state.0))
        }) {
            continue;
        }
        if let Some(existing) = existing {
            for view in existing.iter() {
                commands.entity(view).despawn();
            }
        }
        cache.prepare(&mut meshes, &mut materials, &assets);
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
                Lid {
                    progress: 0.0,
                    target: 0.0,
                },
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
    owners: Query<Ref<ContainerViewers>>,
    mut removed_users: RemovedComponents<ContainerViewers>,
    mut removed_pairs: RemovedComponents<PairedWith>,
    views: Query<&ChestView>,
    mut lids: Query<&mut Lid>,
) {
    let removed: bevy::platform::collections::HashSet<_> =
        removed_users.read().chain(removed_pairs.read()).collect();
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
                && let Ok(mut lid) = lids.get_mut(view.lid)
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
                lid.target = target;
                commands.entity(view.lid).insert(MovingLid);
            }
        }
    }
}
fn animate_lids(
    commands: ParallelCommands,
    time: Res<Time>,
    mut lids: Query<(Entity, &mut Lid, &mut Transform), With<MovingLid>>,
) {
    lids.par_iter_mut()
        .for_each(|(entity, mut lid, mut transform)| {
            lid.progress = lid
                .progress
                .move_towards(lid.target, time.delta_secs() * 4.0);
            let eased = 1.0 - (1.0 - lid.progress).powi(3);
            transform.rotation = Quat::from_rotation_x(-eased * std::f32::consts::FRAC_PI_2);
            if lid.progress == lid.target {
                commands.command_scope(|mut commands| {
                    commands.entity(entity).remove::<MovingLid>();
                });
            }
        });
}
fn clear_updates(mut views: ResMut<ViewQueue>, mut lights: ResMut<LightQueue>) {
    views.clear();
    lights.clear();
}
