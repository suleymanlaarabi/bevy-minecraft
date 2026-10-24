use super::{
    ChunkVoxels, SetVoxel, VoxelChunk, VoxelSettings, VoxelViewer,
    generation::WorldGenerator,
    material::{TexturePack, VoxelMaterial, WaterMaterial, WaterMaterialExtension, block_texture},
    meshing::{ChunkNeighborhood, build_sections},
    streaming::{AwaitingNeighbors, ChunkIndex, StoredChunks},
};
use avian3d::prelude::*;
use bevy::{
    light::{NotShadowCaster, NotShadowReceiver},
    platform::collections::HashSet,
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
    // Advancing revisions also stops pending builds from restoring the previous mapping.
    for mut chunk in &mut chunks {
        let all = chunk.all_sections();
        chunk.mark_sections(all);
    }
}

/// Render sections of a chunk: one child entity per non-empty mesh.
#[derive(Component, Default)]
pub(crate) struct ChunkSections {
    terrain: Vec<Option<Entity>>,
    water: Vec<Option<Entity>>,
    /// Neighbors (bit `slot` of a 3x3 grid) absent from the last applied build.
    pub(crate) missing_neighbors: u16,
}

impl ChunkSections {
    #[cfg(feature = "dev")]
    pub(crate) fn mesh_count(&self) -> usize {
        self.terrain.iter().chain(&self.water).flatten().count()
    }
}

/// GPU-ready meshes of one section, built off the main thread.
struct BuiltSection {
    index: usize,
    terrain: Option<Mesh>,
    water: Option<Mesh>,
    casts_shadow: bool,
}

struct BuildOutput {
    sections: Vec<BuiltSection>,
    missing_neighbors: u16,
    #[cfg(feature = "dev")]
    meshing_elapsed: Duration,
    #[cfg(feature = "dev")]
    counts: (usize, usize),
}

#[derive(Component)]
pub(crate) struct ChunkBuild {
    task: Task<BuildOutput>,
    revision: u64,
}

#[derive(Component)]
pub(crate) struct ColliderBuild {
    task: Task<Option<Collider>>,
    content_revision: u64,
}

/// Concurrent mesh tasks: enough to fill the compute pool without hoarding stale work.
fn max_pending_builds() -> usize {
    2 * bevy::tasks::available_parallelism().max(2)
}

/// Squared distance to the viewer, with coordinates as a deterministic tie-break.
type DistanceKey = (i32, i32, i32);

#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct NeedsChunkBuild;

/// Marks chunks rebuilt because of a player edit; they skip the streaming limits.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct EditedChunk;

type ChangedChunks<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static ChunkVoxels),
    (
        Changed<ChunkVoxels>,
        Without<NeedsChunkBuild>,
        Without<AwaitingNeighbors>,
    ),
>;
type WaitingChunks<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static VoxelChunk,
        &'static ChunkVoxels,
        Has<EditedChunk>,
    ),
    (With<NeedsChunkBuild>, Without<ChunkBuild>),
>;

pub(crate) fn mark_changed_chunks(mut commands: Commands, chunks: ChangedChunks) {
    for (entity, voxels) in &chunks {
        if voxels.dirty_sections != 0 {
            commands.entity(entity).insert(NeedsChunkBuild);
        }
    }
}

/// The 3x3 slot of a neighbor offset, matching `ChunkNeighborhood`.
pub(crate) fn neighbor_slot(offset: IVec2) -> usize {
    ((offset.x + 1) + 3 * (offset.y + 1)) as usize
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

    // Neighbors read this border directly; only their meshes need refreshing.
    for offset in border_neighbors(local, settings.chunk_size) {
        if let Some(entity) = index.get(&(chunk + offset))
            && let Ok(mut voxels) = chunks.get_mut(*entity)
        {
            let sections = voxels.sections_around(local.y);
            voxels.mark_sections(sections);
            commands.entity(*entity).try_insert(EditedChunk);
        }
    }
}

/// Neighbor chunks whose meshes sample this cell (faces or corner light).
fn border_neighbors(local: IVec3, size: i32) -> impl Iterator<Item = IVec2> {
    let side = |value: i32| match value {
        0 => [-1, 0],
        value if value == size - 1 => [1, 0],
        _ => [0, 0],
    };
    let (xs, zs) = (side(local.x), side(local.z));
    let mut offsets = Vec::with_capacity(3);
    for x in [0, xs[0]] {
        for z in [0, zs[0]] {
            if (x, z) != (0, 0) && !offsets.contains(&IVec2::new(x, z)) {
                offsets.push(IVec2::new(x, z));
            }
        }
    }
    offsets.into_iter()
}

pub(crate) fn cleanup_removed_chunk(
    event: On<Remove<ChunkVoxels>>,
    chunks: Query<(&VoxelChunk, &ChunkVoxels)>,
    mut index: ResMut<ChunkIndex>,
    mut stored: ResMut<StoredChunks>,
) {
    if let Ok((chunk, voxels)) = chunks.get(event.entity) {
        index.remove(&chunk.position);
        if voxels.modified {
            stored.insert(chunk.position, voxels.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn start_changed_builds(
    mut commands: Commands,
    texture_pack: Res<TexturePack>,
    settings: Res<VoxelSettings>,
    index: Res<ChunkIndex>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    builds: Query<(), With<ChunkBuild>>,
    chunks: WaitingChunks,
    all: Query<&ChunkVoxels>,
    mut candidates: Local<Vec<(DistanceKey, Entity)>>,
) {
    if chunks.is_empty() {
        return;
    }
    let available = max_pending_builds().saturating_sub(builds.iter().count());
    let center = viewer.map_or(IVec2::ZERO, |viewer| settings.chunk_at(viewer.translation));
    let pool = AsyncComputeTaskPool::get();
    let neighborhood = |position: IVec2, voxels: &ChunkVoxels| {
        ChunkNeighborhood::new(voxels, |offset| {
            index
                .get(&(position + offset))
                .and_then(|&entity| all.get(entity).ok())
        })
    };
    candidates.clear();
    for (entity, chunk, voxels, edited) in &chunks {
        if voxels.dirty_sections == 0 {
            commands.entity(entity).remove::<NeedsChunkBuild>();
        } else if edited {
            // Edits are rare and visible right away, so they never wait for a slot.
            let neighborhood = neighborhood(chunk.position, voxels);
            commands
                .entity(entity)
                .insert(spawn_chunk_build(pool, neighborhood, voxels, &texture_pack));
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
        let Ok((_, chunk, voxels, _)) = chunks.get(entity) else {
            continue;
        };
        let neighborhood = neighborhood(chunk.position, voxels);
        commands
            .entity(entity)
            .insert(spawn_chunk_build(pool, neighborhood, voxels, &texture_pack));
    }
}

fn spawn_chunk_build(
    pool: &AsyncComputeTaskPool,
    neighborhood: ChunkNeighborhood,
    voxels: &ChunkVoxels,
    texture_pack: &TexturePack,
) -> ChunkBuild {
    let sections = voxels.dirty_sections;
    let revision = voxels.revision;
    let texture_pack = texture_pack.clone();
    let task = pool.spawn(async move {
        #[cfg(feature = "dev")]
        let meshing_started = Instant::now();
        let sections = build_sections(&neighborhood, sections, &texture_pack);
        #[cfg(feature = "dev")]
        let counts = sections
            .iter()
            .flat_map(|section| section.terrain.iter().chain(&section.water))
            .map(|geometry| geometry.counts())
            .fold((0, 0), |sum, counts| (sum.0 + counts.0, sum.1 + counts.1));
        let sections = sections
            .into_iter()
            .map(|section| BuiltSection {
                index: section.index,
                casts_shadow: section.casts_shadow,
                terrain: section.terrain.map(|geometry| geometry.into_mesh()),
                water: section.water.map(|geometry| geometry.into_mesh()),
            })
            .collect();
        BuildOutput {
            sections,
            missing_neighbors: neighborhood.missing_neighbors(),
            #[cfg(feature = "dev")]
            meshing_elapsed: meshing_started.elapsed(),
            #[cfg(feature = "dev")]
            counts,
        }
    });
    ChunkBuild { task, revision }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_builds(
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    assets: Option<Res<VoxelAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunks: Query<(
        Entity,
        &mut ChunkVoxels,
        &mut ChunkBuild,
        Option<&mut ChunkSections>,
        Has<EditedChunk>,
    )>,
    #[cfg(feature = "dev")] mut diagnostics: Option<ResMut<super::diagnostics::VoxelDiagnostics>>,
) {
    let mut completed = 0;
    for (entity, mut voxels, mut build, sections, edited) in &mut chunks {
        if completed >= settings.spawn_budget_per_frame && !edited {
            continue;
        }
        let Some(output) = check_ready(&mut build.task) else {
            continue;
        };
        completed += 1;
        commands.entity(entity).remove::<ChunkBuild>();
        if build.revision != voxels.revision {
            // Changed while meshing: its dirty sections stay queued for a retry.
            continue;
        }
        #[cfg(feature = "dev")]
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            let (vertices, triangles) = output.counts;
            diagnostics.record_build(output.meshing_elapsed, Duration::ZERO, vertices, triangles);
        }
        let mut created = None;
        let entities = match sections {
            Some(sections) => sections.into_inner(),
            None => created.insert(ChunkSections::default()),
        };
        let count = voxels.section_count();
        entities.terrain.resize(count, None);
        entities.water.resize(count, None);
        entities.missing_neighbors = output.missing_neighbors;
        if let Some(assets) = assets.as_deref() {
            for section in output.sections {
                let y = (section.index as i32 * super::data::SECTION_HEIGHT) as f32;
                let terrain = section.terrain.map(|mesh| meshes.add(mesh));
                let water = section.water.map(|mesh| meshes.add(mesh));
                let slot = &mut entities.terrain[section.index];
                update_section(&mut commands, slot, terrain, |commands, mesh| {
                    commands
                        .spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.terrain.clone()),
                            Transform::from_xyz(0.0, y, 0.0),
                            ChildOf(entity),
                        ))
                        .id()
                });
                if let Some(terrain) = *slot {
                    let mut terrain = commands.entity(terrain);
                    if section.casts_shadow {
                        terrain.try_remove::<NotShadowCaster>();
                    } else {
                        terrain.try_insert(NotShadowCaster);
                    }
                }
                update_section(
                    &mut commands,
                    &mut entities.water[section.index],
                    water,
                    |commands, mesh| {
                        commands
                            .spawn((
                                Mesh3d(mesh),
                                MeshMaterial3d(assets.water.clone()),
                                Transform::from_xyz(0.0, y, 0.0),
                                NotShadowCaster,
                                NotShadowReceiver,
                                ChildOf(entity),
                            ))
                            .id()
                    },
                );
            }
        }
        if let Some(sections) = created {
            commands.entity(entity).try_insert(sections);
        }
        voxels.bypass_change_detection().dirty_sections = 0;
        commands
            .entity(entity)
            .remove::<(NeedsChunkBuild, EditedChunk)>();
    }
}

/// Swaps a section's mesh, spawning or despawning its entity as it gains or loses faces.
fn update_section(
    commands: &mut Commands,
    slot: &mut Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    spawn: impl FnOnce(&mut Commands, Handle<Mesh>) -> Entity,
) {
    match (mesh, *slot) {
        (Some(mesh), Some(entity)) => {
            commands.entity(entity).try_insert(Mesh3d(mesh));
        }
        (Some(mesh), None) => *slot = Some(spawn(commands, mesh)),
        (None, Some(entity)) => {
            commands.entity(entity).try_despawn();
            *slot = None;
        }
        (None, None) => {}
    }
}

/// Chunks within this Chebyshev distance of a moving body get colliders.
const COLLIDER_RADIUS: i32 = 2;
/// Colliders are dropped only beyond this distance, so bodies near a border do not churn.
const COLLIDER_KEEP_RADIUS: i32 = 4;

/// Collision only exists around moving bodies (player, dropped items...).
pub(crate) fn request_chunk_colliders(
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    index: Res<ChunkIndex>,
    bodies: Query<(&GlobalTransform, &RigidBody)>,
    chunks: Query<(&ChunkVoxels, Has<Collider>, Has<ColliderBuild>)>,
    loaded: Query<(Entity, &VoxelChunk), Or<(With<Collider>, With<ColliderBuild>)>>,
    mut near: Local<HashSet<IVec2>>,
    mut keep: Local<HashSet<IVec2>>,
) {
    near.clear();
    keep.clear();
    for (transform, body) in &bodies {
        if body.is_static() {
            continue;
        }
        let center = settings.chunk_at(transform.translation());
        for x in -COLLIDER_KEEP_RADIUS..=COLLIDER_KEEP_RADIUS {
            for z in -COLLIDER_KEEP_RADIUS..=COLLIDER_KEEP_RADIUS {
                let position = center + IVec2::new(x, z);
                keep.insert(position);
                if x.abs().max(z.abs()) <= COLLIDER_RADIUS {
                    near.insert(position);
                }
            }
        }
    }
    for (entity, chunk) in &loaded {
        if !keep.contains(&chunk.position) {
            commands
                .entity(entity)
                .try_remove::<(Collider, ColliderBuild)>();
        }
    }
    let pool = AsyncComputeTaskPool::get();
    for position in near.iter() {
        let Some(&entity) = index.get(position) else {
            continue;
        };
        let Ok((voxels, has_collider, building)) = chunks.get(entity) else {
            continue;
        };
        if building || (has_collider && !voxels.collider_dirty) {
            continue;
        }
        let snapshot = voxels.clone();
        commands.entity(entity).try_insert(ColliderBuild {
            content_revision: voxels.content_revision,
            task: pool.spawn(async move {
                let solids = snapshot.solid_positions();
                (!solids.is_empty()).then(|| Collider::voxels(Vec3::ONE, &solids))
            }),
        });
    }
}

pub(crate) fn poll_chunk_colliders(
    mut commands: Commands,
    mut chunks: Query<(Entity, &mut ChunkVoxels, &mut ColliderBuild)>,
) {
    for (entity, mut voxels, mut build) in &mut chunks {
        let Some(collider) = check_ready(&mut build.task) else {
            continue;
        };
        let mut entity_commands = commands.entity(entity);
        entity_commands.remove::<ColliderBuild>();
        if build.content_revision != voxels.content_revision {
            continue;
        }
        voxels.bypass_change_detection().collider_dirty = false;
        match collider {
            Some(collider) => entity_commands.try_insert(collider),
            None => entity_commands.try_remove::<Collider>(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::VoxelKind;
    use bevy::ecs::system::RunSystemOnce;
    use std::time::{Duration, Instant};

    #[test]
    fn border_edits_refresh_cardinal_and_diagonal_neighbors() {
        let corner: Vec<_> = border_neighbors(IVec3::new(0, 5, 15), 16).collect();
        assert_eq!(corner.len(), 3);
        assert!(corner.contains(&IVec2::new(-1, 0)));
        assert!(corner.contains(&IVec2::new(0, 1)));
        assert!(corner.contains(&IVec2::new(-1, 1)));
        assert_eq!(border_neighbors(IVec3::new(4, 5, 7), 16).count(), 0);
    }

    #[test]
    fn builds_clear_dirty_sections_only_for_the_current_revision() {
        let mut app = App::new();
        app.add_plugins(TaskPoolPlugin::default())
            .insert_resource(VoxelSettings::default())
            .insert_resource(TexturePack::load(super::super::TexturePackId::default()))
            .init_resource::<ChunkIndex>()
            .init_resource::<Assets<Mesh>>();
        let world = app.world_mut();
        let entity = world
            .spawn((
                VoxelChunk {
                    position: IVec2::ZERO,
                },
                ChunkVoxels::generated(1, 32, vec![VoxelKind::Stone; 32]),
                NeedsChunkBuild,
            ))
            .id();
        world.resource_mut::<ChunkIndex>().insert(IVec2::ZERO, entity);
        world.run_system_once(start_changed_builds).unwrap();
        assert!(world.get::<ChunkBuild>(entity).is_some());
        // An edit while meshing makes the result stale.
        world
            .get_mut::<ChunkVoxels>(entity)
            .unwrap()
            .set(IVec3::new(0, 20, 0), VoxelKind::Air);
        let poll = |world: &mut World| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while world.get::<ChunkBuild>(entity).is_some() {
                assert!(Instant::now() < deadline, "chunk build did not finish");
                world.run_system_once(poll_builds).unwrap();
                std::thread::yield_now();
            }
        };
        poll(world);
        assert_ne!(world.get::<ChunkVoxels>(entity).unwrap().dirty_sections, 0);
        world.run_system_once(start_changed_builds).unwrap();
        poll(world);
        assert_eq!(world.get::<ChunkVoxels>(entity).unwrap().dirty_sections, 0);
        assert!(world.get::<NeedsChunkBuild>(entity).is_none());
        assert_eq!(
            world.get::<ChunkSections>(entity).unwrap().missing_neighbors.count_ones(),
            8
        );
    }
}
