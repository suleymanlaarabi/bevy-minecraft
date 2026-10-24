use super::{
    ChunkVoxels, SetVoxel, VoxelSettings, VoxelViewer,
    data::COLLIDER_CHANGED,
    generation::WorldGenerator,
    material::{TexturePack, VoxelMaterial, WaterMaterial, WaterMaterialExtension, block_texture},
    meshing::{ChunkMeshes, build_chunk_mesh},
    regions::RenderRegions,
    streaming::{ChunkIndex, StoredChunks},
};
use avian3d::prelude::*;
use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
#[cfg(feature = "dev")]
use std::time::{Duration, Instant};
#[derive(Resource)]
pub(crate) struct VoxelAssets {
    pub(crate) terrain: Handle<VoxelMaterial>,
    pub(crate) water: Handle<WaterMaterial>,
}

pub(crate) fn prepare_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    settings: Option<Res<crate::settings::GraphicsSettings>>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
) {
    let pack = TexturePack::load(
        settings
            .as_deref()
            .map_or(crate::voxel::TexturePackId::default(), |settings| {
                settings.texture_pack
            }),
    );
    let blocks = block_texture(&asset_server, &pack.texture_path, pack.layers);
    commands.insert_resource(pack);
    commands.insert_resource(VoxelAssets {
        terrain: voxel_materials.add(VoxelMaterial { blocks }),
        water: water_materials.add(WaterMaterial {
            base: StandardMaterial {
                base_color: Color::linear_rgba(0.035, 0.30, 0.38, 0.56),
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                perceptual_roughness: 0.08,
                reflectance: 0.75,
                ..default()
            },
            extension: WaterMaterialExtension::default(),
        }),
    });
}

pub(crate) fn apply_texture_pack(
    _insert: On<Insert<crate::settings::GraphicsSettings>>,
    settings: Res<crate::settings::GraphicsSettings>,
    asset_server: Res<AssetServer>,
    mut texture_pack: Option<ResMut<TexturePack>>,
    voxel_assets: Option<ResMut<VoxelAssets>>,
    mut voxel_materials: ResMut<Assets<VoxelMaterial>>,
    mut chunks: Query<&mut ChunkVoxels>,
) {
    let (Some(texture_pack), Some(voxel_assets)) = (texture_pack.as_deref_mut(), voxel_assets)
    else {
        return;
    };
    if texture_pack.texture_path == TexturePack::load(settings.texture_pack).texture_path {
        return;
    }

    let next_pack = TexturePack::load(settings.texture_pack);
    if let Some(mut material) = voxel_materials.get_mut(&voxel_assets.terrain) {
        material.blocks = block_texture(&asset_server, &next_pack.texture_path, next_pack.layers);
    }
    *texture_pack = next_pack;
    // Layer assignments can differ between Faithful and the legacy packs.
    // Advance revisions so pending builds cannot restore the previous mapping.
    for mut chunk in &mut chunks {
        chunk.changes |= super::data::MESH_CHANGED;
        chunk.revision = chunk.revision.wrapping_add(1);
    }
}

pub(crate) enum ColliderUpdate {
    Keep,
    Replace(Collider),
    Remove,
}

impl ColliderUpdate {
    pub(crate) fn apply(self, commands: &mut Commands, entity: Entity) {
        match self {
            Self::Keep => {}
            Self::Replace(collider) => {
                commands.entity(entity).try_insert(collider);
            }
            Self::Remove => {
                commands.entity(entity).try_remove::<Collider>();
            }
        }
    }
}
struct BuildOutput {
    meshes: ChunkMeshes,
    collider: ColliderUpdate,
    #[cfg(feature = "dev")]
    meshing_elapsed: Duration,
    #[cfg(feature = "dev")]
    collider_elapsed: Duration,
    #[cfg(feature = "dev")]
    vertices: usize,
    #[cfg(feature = "dev")]
    triangles: usize,
}
#[derive(Component)]
pub(crate) struct ChunkBuild {
    task: Task<BuildOutput>,
    revision: u64,
}

const MAX_PENDING_BUILDS: usize = 16;

/// Squared distance to the viewer, with coordinates as a deterministic tie-break.
type DistanceKey = (i32, i32, i32);

#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct NeedsChunkBuild;

/// Marks chunks rebuilt because of a player edit; they skip the streaming limits.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct EditedChunk;

type ChangedChunks<'w, 's> =
    Query<'w, 's, (Entity, &'static ChunkVoxels), (Changed<ChunkVoxels>, Without<NeedsChunkBuild>)>;
type WaitingChunks<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static super::VoxelChunk,
        &'static ChunkVoxels,
        Has<EditedChunk>,
    ),
    (With<NeedsChunkBuild>, Without<ChunkBuild>),
>;

pub(crate) fn mark_changed_chunks(mut commands: Commands, chunks: ChangedChunks) {
    for (entity, voxels) in &chunks {
        if voxels.changes != 0 {
            commands.entity(entity).insert(NeedsChunkBuild);
        }
    }
}

fn build_revision_is_current(build_revision: u64, voxel_revision: u64) -> bool {
    build_revision == voxel_revision
}

pub(crate) fn set_voxel(
    event: On<SetVoxel>,
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    generator: Res<WorldGenerator>,
    index: Res<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
    mut chunks: Query<&mut ChunkVoxels>,
) {
    let (chunk, local) = settings.split_world_position(event.world_position);
    let changed = if let Some(entity) = index.get(&chunk) {
        let Ok(mut voxels) = chunks.get_mut(*entity) else {
            return;
        };
        let changed = voxels.bypass_change_detection().set(local, event.kind);
        if changed {
            voxels.set_changed();
            commands.entity(*entity).try_insert(EditedChunk);
        }
        changed
    } else {
        stored
            .entry(chunk)
            .or_insert_with(|| super::generation::generate_chunk(chunk, &settings, &generator))
            .set(local, event.kind)
    };
    if !changed {
        return;
    }

    for (neighbor_offset, neighbor_local) in border_halos(local, settings.chunk_size) {
        let position = chunk + neighbor_offset;
        if let Some(entity) = index.get(&position)
            && let Ok(mut voxels) = chunks.get_mut(*entity)
        {
            let changed = voxels
                .bypass_change_detection()
                .set_halo(neighbor_local, event.kind);
            if changed {
                voxels.set_changed();
                commands.entity(*entity).try_insert(EditedChunk);
            }
        } else if let Some(voxels) = stored.get_mut(&position) {
            voxels.set_halo(neighbor_local, event.kind);
        }
    }
}

fn border_halos(local: IVec3, size: i32) -> impl Iterator<Item = (IVec2, IVec3)> {
    [
        (local.x == 0).then_some((IVec2::NEG_X, IVec3::new(size, local.y, local.z))),
        (local.x == size - 1).then_some((IVec2::X, IVec3::new(-1, local.y, local.z))),
        (local.z == 0).then_some((IVec2::NEG_Y, IVec3::new(local.x, local.y, size))),
        (local.z == size - 1).then_some((IVec2::Y, IVec3::new(local.x, local.y, -1))),
    ]
    .into_iter()
    .flatten()
}

pub(crate) fn cleanup_removed_chunk(
    event: On<Remove<ChunkVoxels>>,
    chunks: Query<(&super::VoxelChunk, &ChunkVoxels)>,
    mut index: ResMut<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
    mut render_regions: ResMut<RenderRegions>,
) {
    if let Ok((chunk, voxels)) = chunks.get(event.entity) {
        index.remove(&chunk.position);
        render_regions.remove(chunk.position);
        if voxels.modified {
            stored.insert(chunk.position, voxels.clone());
        }
    }
}

pub(crate) fn start_changed_builds(
    mut commands: Commands,
    texture_pack: Res<TexturePack>,
    settings: Res<VoxelSettings>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    builds: Query<(), With<ChunkBuild>>,
    chunks: WaitingChunks,
    mut candidates: Local<Vec<(DistanceKey, Entity)>>,
) {
    if chunks.is_empty() {
        return;
    }
    let available = MAX_PENDING_BUILDS
        .saturating_sub(builds.iter().count())
        .min(settings.spawn_budget_per_frame);
    let center = viewer.map_or(IVec2::ZERO, |viewer| settings.chunk_at(viewer.translation));
    let pool = AsyncComputeTaskPool::get();
    candidates.clear();
    for (entity, chunk, voxels, edited) in &chunks {
        if voxels.changes == 0 {
            commands.entity(entity).remove::<NeedsChunkBuild>();
        } else if edited {
            // Edits are rare and visible right away, so they never wait for a slot.
            commands
                .entity(entity)
                .insert(spawn_chunk_build(pool, voxels, &texture_pack));
        } else if available > 0 {
            let position = chunk.position;
            candidates.push((
                (position.distance_squared(center), position.x, position.y),
                entity,
            ));
        }
    }
    // Only the nearest `available` chunks start, so a partial selection is enough.
    if available < candidates.len() {
        candidates.select_nth_unstable(available);
        candidates.truncate(available);
    }
    for &(_, entity) in candidates.iter() {
        let Ok((_, _, voxels, _)) = chunks.get(entity) else {
            continue;
        };
        commands
            .entity(entity)
            .insert(spawn_chunk_build(pool, voxels, &texture_pack));
    }
}

fn spawn_chunk_build(
    pool: &AsyncComputeTaskPool,
    voxels: &ChunkVoxels,
    texture_pack: &TexturePack,
) -> ChunkBuild {
    let changes = voxels.changes;
    // Keep dirty outputs until a matching revision is actually applied.
    let snapshot = voxels.clone();
    let revision = snapshot.revision;
    let texture_pack = texture_pack.clone();
    let task = pool.spawn(async move {
        #[cfg(feature = "dev")]
        let meshing_started = Instant::now();
        let meshes = build_chunk_mesh(&snapshot, &texture_pack);
        #[cfg(feature = "dev")]
        let meshing_elapsed = meshing_started.elapsed();
        #[cfg(feature = "dev")]
        let (vertices, triangles) = meshes.geometry_counts();
        #[cfg(feature = "dev")]
        let collider_started = Instant::now();
        let collider = if changes & COLLIDER_CHANGED == 0 {
            ColliderUpdate::Keep
        } else {
            let solids = snapshot.solid_positions();
            if solids.is_empty() {
                ColliderUpdate::Remove
            } else {
                ColliderUpdate::Replace(Collider::voxels(Vec3::ONE, &solids))
            }
        };
        BuildOutput {
            meshes,
            collider,
            #[cfg(feature = "dev")]
            meshing_elapsed,
            #[cfg(feature = "dev")]
            collider_elapsed: collider_started.elapsed(),
            #[cfg(feature = "dev")]
            vertices,
            #[cfg(feature = "dev")]
            triangles,
        }
    });
    ChunkBuild { task, revision }
}

pub(crate) fn poll_builds(
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    mut chunks: Query<(
        Entity,
        &super::VoxelChunk,
        &mut ChunkVoxels,
        &mut ChunkBuild,
        Has<EditedChunk>,
    )>,
    mut render_regions: ResMut<RenderRegions>,
    #[cfg(feature = "dev")] mut diagnostics: Option<ResMut<super::diagnostics::VoxelDiagnostics>>,
) {
    let mut completed = 0;
    for (entity, chunk, mut voxels, mut build, edited) in &mut chunks {
        if completed >= settings.spawn_budget_per_frame && !edited {
            continue;
        }
        let Some(output) = check_ready(&mut build.task) else {
            continue;
        };
        completed += 1;
        if !build_revision_is_current(build.revision, voxels.revision) {
            commands.entity(entity).remove::<ChunkBuild>();
            continue;
        }
        let BuildOutput {
            meshes: ChunkMeshes(terrain, next_water),
            collider,
            #[cfg(feature = "dev")]
            meshing_elapsed,
            #[cfg(feature = "dev")]
            collider_elapsed,
            #[cfg(feature = "dev")]
            vertices,
            #[cfg(feature = "dev")]
            triangles,
        } = output;
        #[cfg(feature = "dev")]
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.record_build(meshing_elapsed, collider_elapsed, vertices, triangles);
        }
        let meshes = ChunkMeshes(terrain, next_water);
        if let Some(collider) =
            render_regions.insert_with_collider(chunk.position, meshes, edited, entity, collider)
        {
            collider.apply(&mut commands, entity);
        }
        voxels.bypass_change_detection().changes = 0;
        commands
            .entity(entity)
            .remove::<(ChunkBuild, NeedsChunkBuild, EditedChunk)>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use std::time::{Duration, Instant};

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(TaskPoolPlugin::default())
            .insert_resource(VoxelSettings {
                spawn_budget_per_frame: 32,
                ..default()
            })
            .insert_resource(TexturePack::load(super::super::TexturePackId::default()))
            .init_resource::<RenderRegions>();
        app
    }

    fn chunk(world: &mut World, position: IVec2) -> Entity {
        world
            .spawn((
                super::super::VoxelChunk { position },
                ChunkVoxels::generated(
                    1,
                    1,
                    vec![super::super::VoxelKind::Stone],
                    vec![super::super::VoxelKind::Air; 4],
                ),
            ))
            .id()
    }

    fn poll_until_finished(world: &mut World, entity: Entity) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while world.get::<ChunkBuild>(entity).is_some() {
            assert!(Instant::now() < deadline, "chunk build did not finish");
            world.run_system_once(poll_builds).unwrap();
            std::thread::yield_now();
        }
    }

    fn edited_chunk_world(world: &mut World) -> Entity {
        world.init_resource::<ChunkIndex>();
        world.init_resource::<StoredChunks>();
        world.init_resource::<RenderRegions>();
        world.init_resource::<super::super::streaming::PendingChunks>();
        world.init_resource::<super::super::StreamState>();
        world.add_observer(cleanup_removed_chunk);
        let mut voxels = ChunkVoxels::generated(
            1,
            1,
            vec![super::super::VoxelKind::Stone],
            vec![super::super::VoxelKind::Air; 4],
        );
        voxels.set(IVec3::ZERO, super::super::VoxelKind::Air);
        let entity = world
            .spawn((
                super::super::VoxelChunk {
                    position: IVec2::ZERO,
                },
                voxels,
            ))
            .id();
        world
            .resource_mut::<ChunkIndex>()
            .insert(IVec2::ZERO, entity);
        entity
    }

    #[test]
    fn unloading_an_edited_chunk_keeps_its_edits() {
        let mut world = World::new();
        let entity = edited_chunk_world(&mut world);
        world.despawn(entity);
        assert!(world.resource::<ChunkIndex>().is_empty());
        assert_eq!(world.resource::<StoredChunks>().len(), 1);
    }

    #[test]
    fn session_teardown_leaves_no_stored_edits_or_dirty_regions() {
        let mut world = World::new();
        edited_chunk_world(&mut world);
        world
            .resource_mut::<RenderRegions>()
            .insert(IVec2::ZERO, ChunkMeshes(default(), None));
        world
            .run_system_once(super::super::cleanup_voxel_world)
            .unwrap();
        assert!(world.resource::<ChunkIndex>().is_empty());
        assert!(world.resource::<StoredChunks>().is_empty());
        assert!(world.resource::<RenderRegions>().is_idle());
    }

    #[test]
    fn edited_chunks_skip_the_streaming_build_limit() {
        let mut app = app();
        let world = app.world_mut();
        for _ in 0..MAX_PENDING_BUILDS {
            world.spawn(ChunkBuild {
                revision: 0,
                task: AsyncComputeTaskPool::get().spawn(std::future::pending()),
            });
        }
        let streamed = chunk(world, IVec2::ZERO);
        let edited = chunk(world, IVec2::X);
        world.entity_mut(edited).insert(EditedChunk);
        world.run_system_once(mark_changed_chunks).unwrap();
        world.run_system_once(start_changed_builds).unwrap();
        assert!(world.get::<ChunkBuild>(streamed).is_none());
        assert!(world.get::<ChunkBuild>(edited).is_some());
        poll_until_finished(world, edited);
        assert!(world.get::<EditedChunk>(edited).is_none());
        // The edit collider waits for the region merge that draws the same edit.
        assert!(world.get::<Collider>(edited).is_none());
    }

    #[test]
    fn unchanged_chunks_do_not_use_build_slots() {
        let mut app = app();
        let world = app.world_mut();
        world.resource_mut::<VoxelSettings>().spawn_budget_per_frame = 1;
        let near = chunk(world, IVec2::ZERO);
        let far = chunk(world, IVec2::splat(20));
        world.run_system_once(mark_changed_chunks).unwrap();
        world.get_mut::<ChunkVoxels>(near).unwrap().changes = 0;
        world.run_system_once(start_changed_builds).unwrap();
        assert!(world.get::<NeedsChunkBuild>(near).is_none());
        assert!(world.get::<ChunkBuild>(far).is_some());
    }

    #[test]
    fn stale_build_keeps_collision_dirty_until_retry_applies() {
        let mut app = app();
        let world = app.world_mut();
        let entity = chunk(world, IVec2::ZERO);
        world.run_system_once(mark_changed_chunks).unwrap();
        world.run_system_once(start_changed_builds).unwrap();
        world
            .get_mut::<ChunkVoxels>(entity)
            .unwrap()
            .set_halo(IVec3::NEG_X, super::super::VoxelKind::Stone);
        poll_until_finished(world, entity);
        assert!(world.get::<NeedsChunkBuild>(entity).is_some());
        assert_ne!(
            world.get::<ChunkVoxels>(entity).unwrap().changes & COLLIDER_CHANGED,
            0
        );
        assert!(world.get::<Collider>(entity).is_none());
        world.run_system_once(start_changed_builds).unwrap();
        poll_until_finished(world, entity);
        assert!(world.get::<Collider>(entity).is_some());
        assert!(world.get::<NeedsChunkBuild>(entity).is_none());
        assert_eq!(world.get::<ChunkVoxels>(entity).unwrap().changes, 0);
    }

    #[test]
    fn capacity_limit_preserves_queued_chunks_until_a_slot_is_free() {
        let mut app = app();
        let world = app.world_mut();
        let blockers: Vec<_> = (0..MAX_PENDING_BUILDS)
            .map(|_| {
                world
                    .spawn(ChunkBuild {
                        revision: 0,
                        task: AsyncComputeTaskPool::get().spawn(std::future::pending()),
                    })
                    .id()
            })
            .collect();
        let entity = chunk(world, IVec2::ZERO);
        world.run_system_once(mark_changed_chunks).unwrap();
        world.run_system_once(start_changed_builds).unwrap();
        assert!(world.get::<NeedsChunkBuild>(entity).is_some());
        assert!(world.get::<ChunkBuild>(entity).is_none());
        world.despawn(blockers[0]);
        world.run_system_once(start_changed_builds).unwrap();
        assert!(world.get::<ChunkBuild>(entity).is_some());
        assert_eq!(
            world.query::<&ChunkBuild>().iter(world).count(),
            MAX_PENDING_BUILDS
        );
    }

    #[test]
    fn nearest_chunk_is_started_first() {
        let mut app = app();
        let world = app.world_mut();
        world.resource_mut::<VoxelSettings>().spawn_budget_per_frame = 1;
        let far = chunk(world, IVec2::splat(20));
        let near = chunk(world, IVec2::ZERO);
        world.run_system_once(mark_changed_chunks).unwrap();
        world.run_system_once(start_changed_builds).unwrap();
        assert!(world.get::<ChunkBuild>(near).is_some());
        assert!(world.get::<ChunkBuild>(far).is_none());
        assert!(world.get::<NeedsChunkBuild>(far).is_some());
    }
}
