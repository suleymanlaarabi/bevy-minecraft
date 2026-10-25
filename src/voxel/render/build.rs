//! Scheduling and application of asynchronous section meshes.

use super::{
    assets::VoxelAssets,
    meshing::{ChunkNeighborhood, build_sections},
};
use crate::voxel::{
    ChunkVoxels, SHADOW_LAYER, VoxelChunk, VoxelViewer,
    coordinates::{SECTION_COUNT, SECTION_HEIGHT, chunk_at},
    edits::EditedChunk,
    streaming::{AwaitingNeighbors, ChunkIndex},
};
use bevy::{
    camera::visibility::{NoCpuCulling, RenderLayers},
    light::NotShadowCaster,
    mesh::Indices,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
#[cfg(feature = "dev")]
use std::time::{Duration, Instant};

/// Render sections of a chunk: one child entity per non-empty mesh.
#[derive(Component, Default)]
pub(crate) struct ChunkSections {
    terrain: Vec<Option<Entity>>,
    cutout: Vec<Option<Entity>>,
    models: Vec<Option<Entity>>,
    models_cutout: Vec<Option<Entity>>,
    translucent: Vec<Option<Entity>>,
    /// Depth-only terrain, seen by the sun's shadow maps and not by cameras.
    shadow: Vec<Option<Entity>>,
    /// Neighbors (bit `slot` of a 3x3 grid) absent from the last applied build.
    pub(crate) missing_neighbors: u16,
}

impl ChunkSections {
    pub(crate) fn mesh_count(&self) -> usize {
        self.terrain
            .iter()
            .chain(&self.cutout)
            .chain(&self.models)
            .chain(&self.models_cutout)
            .chain(&self.translucent)
            .chain(&self.shadow)
            .flatten()
            .count()
    }
}

/// GPU-ready meshes of one section, built off the main thread.
struct BuiltSection {
    index: usize,
    terrain: Option<Mesh>,
    cutout: Option<Mesh>,
    models: Option<Mesh>,
    models_cutout: Option<Mesh>,
    /// Mesh sorted for the view at build time, its quads, and the view key it
    /// was sorted for (`None` without a view).
    translucent: Option<(
        Mesh,
        super::model_mesh::SortData,
        Option<super::sort::SortKey>,
    )>,
    shadow: Option<Mesh>,
}

struct BuildOutput {
    sections: Vec<BuiltSection>,
    bytes: usize,
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

/// Concurrent mesh tasks: enough to fill the compute pool without hoarding stale work.
fn max_pending_builds() -> usize {
    2 * AsyncComputeTaskPool::get().thread_num().max(1)
}

/// Mesh bytes handed to the GPU per frame by streaming; edits are never delayed.
/// Spreads the upload of newly visible terrain over several frames.
const UPLOAD_BUDGET_BYTES: usize = 2 << 20;
/// Section meshes applied per frame by streaming: each one is an asset to prepare
/// and often an entity for the render world to extract and specialize.
const UPLOAD_BUDGET_SECTIONS: usize = 64;

fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.get_vertex_buffer_size()
        + mesh.indices().map_or(0, |indices| match indices {
            Indices::U16(indices) => indices.len() * 2,
            Indices::U32(indices) => indices.len() * 4,
        })
}

/// Squared distance to the viewer, with coordinates as a deterministic tie-break.
type DistanceKey = (i32, i32, i32);

#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct NeedsChunkBuild;

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

pub(crate) fn start_changed_builds(
    mut commands: Commands,
    index: Res<ChunkIndex>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    camera: Option<Single<&GlobalTransform, With<crate::player::PlayerCamera>>>,
    builds: Query<(), With<ChunkBuild>>,
    chunks: WaitingChunks,
    all: Query<&ChunkVoxels>,
    mut candidates: Local<Vec<(DistanceKey, Entity)>>,
) {
    if chunks.is_empty() {
        return;
    }
    let available = max_pending_builds().saturating_sub(builds.iter().count());
    // Grid-cell order depends on the eye, not its orientation.
    let eye = camera
        .map(|camera| camera.translation())
        .or_else(|| viewer.as_ref().map(|viewer| viewer.translation));
    let center = viewer.map_or(IVec2::ZERO, |viewer| chunk_at(viewer.translation));
    let pool = AsyncComputeTaskPool::get();
    let neighborhood = |position: IVec2, voxels: &ChunkVoxels| {
        ChunkNeighborhood::new(voxels, |offset| {
            index
                .get(&(position + offset))
                .and_then(|&entity| all.get(entity).ok())
        })
        .at(position)
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
                .insert(spawn_chunk_build(pool, neighborhood, voxels, eye));
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
            .insert(spawn_chunk_build(pool, neighborhood, voxels, eye));
    }
}

fn spawn_chunk_build(
    pool: &AsyncComputeTaskPool,
    neighborhood: ChunkNeighborhood,
    voxels: &ChunkVoxels,
    eye: Option<Vec3>,
) -> ChunkBuild {
    let sections = voxels.dirty_sections;
    let revision = voxels.revision;
    let task = pool.spawn(async move {
        #[cfg(feature = "dev")]
        let meshing_started = Instant::now();
        let sections = build_sections(&neighborhood, sections);
        #[cfg(feature = "dev")]
        let counts = sections
            .iter()
            .flat_map(|section| {
                section
                    .terrain
                    .iter()
                    .map(|geometry| geometry.counts())
                    .chain(section.cutout.iter().map(|geometry| geometry.counts()))
                    .chain(section.models.iter().map(|geometry| geometry.counts()))
                    .chain(
                        section
                            .models_cutout
                            .iter()
                            .map(|geometry| geometry.counts()),
                    )
                    .chain(section.translucent.iter().map(|geometry| geometry.counts()))
            })
            .fold((0, 0), |sum, counts| (sum.0 + counts.0, sum.1 + counts.1));
        let sections = sections
            .into_iter()
            .map(|section| BuiltSection {
                index: section.index,
                terrain: section.terrain.map(|geometry| geometry.into_mesh()),
                cutout: section.cutout.map(|geometry| geometry.into_mesh()),
                models: section.models.map(|geometry| geometry.into_mesh()),
                models_cutout: section.models_cutout.map(|geometry| geometry.into_mesh()),
                translucent: section.translucent.map(|mut geometry| {
                    // First sort here, off the main thread; `sort` re-sorts later
                    // in place, so the mesh keeps its main-world copy.
                    let origin = (neighborhood.origin
                        + IVec3::Y * (section.index as i32 * SECTION_HEIGHT))
                        .as_vec3();
                    let sort = geometry.sort_data();
                    let key = eye.map(|eye| {
                        geometry.indices = sort.indices(eye - origin);
                        super::sort::sort_key(eye, eye - origin)
                    });
                    (geometry.into_dynamic_mesh(), sort, key)
                }),
                shadow: section.shadow.map(|geometry| geometry.into_mesh()),
            })
            .collect::<Vec<_>>();
        let mut bytes: usize = sections
            .iter()
            .flat_map(|section| [&section.terrain, &section.cutout, &section.shadow])
            .flatten()
            .map(mesh_bytes)
            .sum();
        bytes += sections
            .iter()
            .map(|section| {
                section.models.as_ref().map_or(0, mesh_bytes)
                    + section.models_cutout.as_ref().map_or(0, mesh_bytes)
                    + section
                        .translucent
                        .as_ref()
                        .map_or(0, |(mesh, _, _)| mesh_bytes(mesh))
            })
            .sum::<usize>();
        BuildOutput {
            sections,
            bytes,
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
    assets: Option<Res<VoxelAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunks: Query<(
        Entity,
        &mut ChunkVoxels,
        &mut ChunkBuild,
        Option<&mut ChunkSections>,
        Has<EditedChunk>,
    )>,
    #[cfg(feature = "dev")] mut diagnostics: Option<
        ResMut<crate::voxel::diagnostics::VoxelDiagnostics>,
    >,
) {
    let (mut uploaded, mut meshes_applied) = (0, 0);
    for (entity, mut voxels, mut build, sections, edited) in &mut chunks {
        // Finished builds wait in their task until a later frame has budget.
        if !edited && (uploaded >= UPLOAD_BUDGET_BYTES || meshes_applied >= UPLOAD_BUDGET_SECTIONS)
        {
            continue;
        }
        let Some(output) = check_ready(&mut build.task) else {
            continue;
        };
        uploaded += output.bytes;
        meshes_applied += output.sections.len();
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
        entities.terrain.resize(SECTION_COUNT, None);
        entities.cutout.resize(SECTION_COUNT, None);
        entities.models.resize(SECTION_COUNT, None);
        entities.models_cutout.resize(SECTION_COUNT, None);
        entities.translucent.resize(SECTION_COUNT, None);
        entities.shadow.resize(SECTION_COUNT, None);
        entities.missing_neighbors = output.missing_neighbors;
        if let Some(assets) = assets.as_deref() {
            for section in output.sections {
                let y = (section.index as i32 * SECTION_HEIGHT) as f32;
                let terrain = section.terrain.map(|mesh| meshes.add(mesh));
                let cutout = section.cutout.map(|mesh| meshes.add(mesh));
                let shadow = section.shadow.map(|mesh| meshes.add(mesh));
                let model_mesh = section.models.map(|mesh| meshes.add(mesh));
                let model_cutout = section.models_cutout.map(|mesh| meshes.add(mesh));
                let translucent = section
                    .translucent
                    .map(|(mesh, sort, key)| (meshes.add(mesh), sort, key));
                update_section(
                    &mut commands,
                    &mut entities.models[section.index],
                    model_mesh,
                    entity,
                    y,
                    |mesh| {
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.models.clone()),
                            // Static like terrain: culled on GPU only.
                            NoCpuCulling,
                        )
                    },
                );
                update_section(
                    &mut commands,
                    &mut entities.models_cutout[section.index],
                    model_cutout,
                    entity,
                    y,
                    |mesh| {
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.models_cutout.clone()),
                            NoCpuCulling,
                        )
                    },
                );
                update_section(
                    &mut commands,
                    &mut entities.cutout[section.index],
                    cutout,
                    entity,
                    y,
                    |mesh| {
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.cutout.clone()),
                            // Casts its own alpha-tested shadow.
                            NoCpuCulling,
                        )
                    },
                );
                let transparent_mesh = translucent.as_ref().map(|(mesh, _, _)| mesh.clone());
                update_section(
                    &mut commands,
                    &mut entities.translucent[section.index],
                    transparent_mesh,
                    entity,
                    y,
                    |mesh| {
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.translucent.clone()),
                            NotShadowCaster,
                        )
                    },
                );
                if let (Some(entity), Some((_, sort, key))) =
                    (entities.translucent[section.index], translucent)
                {
                    // A fresh state also drops any sort of the replaced mesh.
                    commands
                        .entity(entity)
                        .insert((sort, super::sort::SortState::new(key)));
                }
                update_section(
                    &mut commands,
                    &mut entities.terrain[section.index],
                    terrain,
                    entity,
                    y,
                    |mesh| {
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.terrain.clone()),
                            // The shadow entity below casts its shadow.
                            NotShadowCaster,
                            // Culled on GPU only: no per-frame CPU work for static terrain.
                            NoCpuCulling,
                        )
                    },
                );
                update_section(
                    &mut commands,
                    &mut entities.shadow[section.index],
                    shadow,
                    entity,
                    y,
                    |mesh| {
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.terrain.clone()),
                            RenderLayers::layer(SHADOW_LAYER),
                            NoCpuCulling,
                        )
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
fn update_section<B: Bundle>(
    commands: &mut Commands,
    slot: &mut Option<Entity>,
    mesh: Option<Handle<Mesh>>,
    chunk: Entity,
    y: f32,
    bundle: impl FnOnce(Handle<Mesh>) -> B,
) {
    match (mesh, *slot) {
        (Some(mesh), Some(entity)) => {
            commands.entity(entity).try_insert(Mesh3d(mesh));
        }
        (Some(mesh), None) => {
            let transform = Transform::from_xyz(0.0, y, 0.0);
            *slot = Some(
                commands
                    .spawn((bundle(mesh), transform, ChildOf(chunk)))
                    .id(),
            );
        }
        (None, Some(entity)) => {
            commands.entity(entity).try_despawn();
            *slot = None;
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::voxel::VoxelKind;
    use bevy::ecs::system::RunSystemOnce;
    use std::time::{Duration, Instant};

    #[test]
    fn builds_clear_dirty_sections_only_for_the_current_revision() {
        let mut app = App::new();
        app.add_plugins(TaskPoolPlugin::default())
            .init_resource::<ChunkIndex>()
            .init_resource::<Assets<Mesh>>();
        let world = app.world_mut();
        let entity = world
            .spawn((
                VoxelChunk {
                    position: IVec2::ZERO,
                },
                ChunkVoxels::filled(VoxelKind::Stone),
                NeedsChunkBuild,
            ))
            .id();
        world
            .resource_mut::<ChunkIndex>()
            .insert(IVec2::ZERO, entity);
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
            world
                .get::<ChunkSections>(entity)
                .unwrap()
                .missing_neighbors
                .count_ones(),
            8
        );
    }
}
