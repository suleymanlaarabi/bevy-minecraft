//! Scheduling and application of asynchronous section meshes.

use super::{
    assets::VoxelAssets,
    meshing::{ChunkNeighborhood, build_sections},
};
use crate::voxel::{
    ChunkVoxels, SHADOW_LAYER, VoxelChunk, VoxelViewer,
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
use hollow_worldgen::coordinates::{SECTION_COUNT, SECTION_HEIGHT, chunk_at};
use std::time::{Duration, Instant};

/// Render sections of a chunk: one child entity per non-empty mesh.
#[derive(Component, Default)]
pub(crate) struct ChunkSections {
    terrain: [Option<Entity>; SECTION_COUNT],
    cutout: [Option<Entity>; SECTION_COUNT],
    models: [Option<Entity>; SECTION_COUNT],
    models_cutout: [Option<Entity>; SECTION_COUNT],
    translucent: [Option<Entity>; SECTION_COUNT],
    /// Depth-only terrain, seen by the sun's shadow maps and not by cameras.
    shadow: [Option<Entity>; SECTION_COUNT],
    /// Neighbors (bit `slot` of a 3x3 grid) absent from the last applied build.
    pub(crate) missing_neighbors: u16,
    pub(super) connectivity: [u64; SECTION_COUNT],
    pub(super) content_revision: u64,
    pub(super) built_sections: u32,
    pub(super) overhanging_sections: u32,
}

impl ChunkSections {
    pub(super) fn camera_entities(&self, section: usize) -> [Option<Entity>; 5] {
        [
            self.terrain[section],
            self.cutout[section],
            self.models[section],
            self.models_cutout[section],
            self.translucent[section],
        ]
    }

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
    connectivity: u64,
    overhang: bool,
    bytes: usize,
    mesh_count: usize,
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
    sections: std::vec::IntoIter<BuiltSection>,
    missing_neighbors: u16,
    #[cfg(feature = "dev")]
    meshing_elapsed: Duration,
    #[cfg(feature = "dev")]
    counts: (usize, usize),
}

#[derive(Component)]
pub(crate) struct ChunkBuild {
    task: Task<BuildOutput>,
    ready: Option<BuildOutput>,
    content_revision: u64,
    revision: u64,
}

/// Concurrent mesh tasks: enough to fill the compute pool without hoarding stale work.
fn max_pending_builds() -> usize {
    2 * AsyncComputeTaskPool::get().thread_num().max(1)
}

/// Mesh bytes handed to the GPU per frame, including player edits.
/// Spreads the upload of newly visible terrain over several frames.
const UPLOAD_BUDGET_BYTES: usize = 2 << 20;
/// Section meshes applied per frame by streaming: each one is an asset to prepare
/// and often an entity for the render world to extract and specialize.
const UPLOAD_BUDGET_MESHES: usize = 32;
const UPLOAD_BUDGET_TIME: Duration = Duration::from_millis(1);

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
        Option<&'static ChunkSections>,
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
    budget: Res<hollow_worldgen::tasks::VoxelTaskBudget>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    camera: Option<Single<&GlobalTransform, With<crate::player::PlayerCamera>>>,
    builds: Query<Has<EditedChunk>, With<ChunkBuild>>,
    chunks: WaitingChunks,
    all: Query<&ChunkVoxels>,
    mut candidates: Local<Vec<((bool, DistanceKey), Entity)>>,
) {
    if chunks.is_empty() {
        return;
    }
    let available = max_pending_builds().saturating_sub(builds.iter().count());
    // Player edits can start even when streaming occupies the ordinary slots.
    let edit_available = AsyncComputeTaskPool::get()
        .thread_num()
        .max(1)
        .saturating_sub(builds.iter().filter(|edited| *edited).count());
    // Grid-cell order depends on the eye, not its orientation.
    let eye = camera
        .map(|camera| camera.translation())
        .or_else(|| viewer.as_ref().map(|viewer| viewer.translation));
    let center = viewer.map_or(IVec2::ZERO, |viewer| chunk_at(viewer.translation));
    let neighborhood = |position: IVec2, voxels: &ChunkVoxels| {
        ChunkNeighborhood::new(voxels, |offset| {
            index
                .get(&(position + offset))
                .and_then(|&entity| all.get(entity).ok())
        })
        .at(position)
    };
    candidates.clear();
    for (entity, chunk, voxels, edited, _) in &chunks {
        if voxels.dirty_sections == 0 {
            commands.entity(entity).remove::<NeedsChunkBuild>();
        } else if available > 0 || (edited && edit_available > 0) {
            let position = chunk.position;
            candidates.push((
                (
                    !edited,
                    (
                        hollow_worldgen::coordinates::chunk_distance_squared(position, center),
                        position.x,
                        position.y,
                    ),
                ),
                entity,
            ));
        }
    }
    let priority = candidates.iter().filter(|(key, _)| !key.0).count();
    let available = available.max(edit_available.min(priority));
    // Only the nearest `available` chunks start, so a partial selection is enough.
    if available < candidates.len() {
        candidates.select_nth_unstable(available);
        candidates.truncate(available);
    }
    for &(priority, entity) in candidates.iter() {
        let Ok((_, chunk, voxels, _, sections)) = chunks.get(entity) else {
            continue;
        };
        let mut neighborhood = neighborhood(chunk.position, voxels);
        if let Some(sections) = sections
            && sections.content_revision == voxels.content_revision
        {
            neighborhood =
                neighborhood.reuse_connectivity(sections.connectivity, sections.built_sections);
        }
        commands.entity(entity).insert(spawn_chunk_build(
            &budget,
            if priority.0 {
                hollow_worldgen::tasks::WorkClass::Streaming
            } else {
                hollow_worldgen::tasks::WorkClass::EditedMesh
            },
            priority.1.0 as u32,
            neighborhood,
            voxels,
            eye,
        ));
    }
}

fn spawn_chunk_build(
    budget: &hollow_worldgen::tasks::VoxelTaskBudget,
    class: hollow_worldgen::tasks::WorkClass,
    distance: u32,
    neighborhood: ChunkNeighborhood,
    voxels: &ChunkVoxels,
    eye: Option<Vec3>,
) -> ChunkBuild {
    let sections = voxels.dirty_sections;
    let revision = voxels.revision;
    let task = budget.spawn(class, distance, async move {
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
        let mut sections = sections
            .into_iter()
            .map(|section| BuiltSection {
                connectivity: section.connectivity,
                overhang: false,
                bytes: 0,
                mesh_count: 0,
                index: section.index,
                terrain: section.terrain.map(|geometry| geometry.into_mesh()),
                cutout: section.cutout.map(|geometry| geometry.into_mesh()),
                models: section.models.map(|geometry| geometry.into_mesh()),
                models_cutout: section.models_cutout.map(|geometry| geometry.into_mesh()),
                translucent: section.translucent.map(|mut geometry| {
                    // First sort runs off the main thread; later sorts patch only GPU indices.
                    let origin = (neighborhood.origin
                        + IVec3::Y * (section.index as i32 * SECTION_HEIGHT))
                        .as_vec3();
                    let sort = geometry.sort_data();
                    let key = eye.map(|eye| {
                        geometry.indices = sort.indices(eye - origin);
                        super::sort::sort_key(eye, eye - origin)
                    });
                    (geometry.into_mesh(), sort, key)
                }),
                shadow: section.shadow.map(|geometry| geometry.into_mesh()),
            })
            .collect::<Vec<_>>();
        for section in &mut sections {
            for mesh in [
                section.terrain.as_ref(),
                section.cutout.as_ref(),
                section.models.as_ref(),
                section.models_cutout.as_ref(),
                section.translucent.as_ref().map(|(mesh, _, _)| mesh),
                section.shadow.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                section.overhang |= mesh.final_aabb.is_none_or(|bounds| {
                    bounds.min.min_element() < 0.0
                        || bounds.max.max_element() > SECTION_HEIGHT as f32
                });
                section.bytes += mesh_bytes(mesh);
                section.mesh_count += 1;
            }
        }
        BuildOutput {
            sections: sections.into_iter(),
            missing_neighbors: neighborhood.missing_neighbors(),
            #[cfg(feature = "dev")]
            meshing_elapsed: meshing_started.elapsed(),
            #[cfg(feature = "dev")]
            counts,
        }
    });
    ChunkBuild {
        task,
        ready: None,
        revision,
        content_revision: voxels.content_revision,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_builds(
    mut commands: Commands,
    assets: Option<Res<VoxelAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    mut topology: Option<ResMut<super::visibility::TopologyVersion>>,
    mut candidates: Local<Vec<((bool, i32, Entity), Entity)>>,
    mut chunks: Query<(
        Entity,
        &VoxelChunk,
        &mut ChunkVoxels,
        &mut ChunkBuild,
        Option<&mut ChunkSections>,
        Has<EditedChunk>,
    )>,
    #[cfg(feature = "dev")] mut diagnostics: Option<
        ResMut<crate::voxel::diagnostics::VoxelDiagnostics>,
    >,
) {
    let started = Instant::now();
    let (mut uploaded, mut meshes_applied, mut applied_sections) = (0usize, 0usize, 0usize);
    let center = viewer.map_or(IVec2::ZERO, |viewer| chunk_at(viewer.translation));
    candidates.clear();
    candidates.extend(chunks.iter().map(|(entity, chunk, _, _, _, edited)| {
        (
            (
                !edited,
                hollow_worldgen::coordinates::chunk_distance_squared(chunk.position, center),
                entity,
            ),
            entity,
        )
    }));
    candidates.sort_unstable();
    for &(_, entity) in candidates.iter() {
        let Ok((_, _, mut voxels, mut build, sections, _)) = chunks.get_mut(entity) else {
            continue;
        };
        if build.revision != voxels.revision {
            // Cancel stale work before charging any upload budget.
            commands.entity(entity).remove::<ChunkBuild>();
            continue;
        }
        if build.ready.is_none() {
            let Some(output) = check_ready(&mut build.task) else {
                continue;
            };
            #[cfg(feature = "dev")]
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                let (vertices, triangles) = output.counts;
                diagnostics.record_build(output.meshing_elapsed, vertices, triangles);
            }
            build.ready = Some(output);
        }
        let output = build.ready.as_mut().expect("completed build");
        if applied_sections > 0
            && let Some(next) = output.sections.as_slice().first()
            && (uploaded.saturating_add(next.bytes) > UPLOAD_BUDGET_BYTES
                || meshes_applied + next.mesh_count > UPLOAD_BUDGET_MESHES
                || started.elapsed() >= UPLOAD_BUDGET_TIME)
        {
            // Borrowing ChunkSections mutably would trigger visibility work even
            // when this build cannot integrate a single section this frame.
            continue;
        }
        let mut created = None;
        let entities = match sections {
            Some(sections) => sections.into_inner(),
            None => created.insert(ChunkSections::default()),
        };
        entities.missing_neighbors = output.missing_neighbors;
        while let Some(next) = output.sections.as_slice().first() {
            // A section is atomic across its material passes. Always allow one
            // section to make progress, even if its geometry exceeds the byte cap.
            if applied_sections > 0
                && (uploaded.saturating_add(next.bytes) > UPLOAD_BUDGET_BYTES
                    || meshes_applied + next.mesh_count > UPLOAD_BUDGET_MESHES
                    || started.elapsed() >= UPLOAD_BUDGET_TIME)
            {
                break;
            }
            let section = output.sections.next().expect("pending section");
            let previous_connectivity = if entities.built_sections & (1 << section.index) != 0 {
                entities.connectivity[section.index]
            } else {
                super::visibility::ALL_CONNECTIONS
            };
            if entities.content_revision == voxels.content_revision
                && previous_connectivity != section.connectivity
                && let Some(version) = topology.as_deref_mut()
            {
                version.invalidate();
            }
            entities.connectivity[section.index] = section.connectivity;
            entities.built_sections |= 1 << section.index;
            let section_bit = 1 << section.index;
            if section.overhang {
                entities.overhanging_sections |= section_bit;
            } else {
                entities.overhanging_sections &= !section_bit;
            }
            uploaded += section.bytes;
            meshes_applied += section.mesh_count;
            applied_sections += 1;
            voxels.bypass_change_detection().dirty_sections &= !(1 << section.index);
            if let Some(assets) = assets.as_deref() {
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
                        .insert(super::uploads::PendingSort(Some((sort, key))));
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
        let complete = output.sections.as_slice().is_empty();
        if complete {
            if entities.content_revision != build.content_revision
                && let Some(version) = topology.as_deref_mut()
            {
                version.invalidate();
            }
            entities.content_revision = build.content_revision;
        }
        if let Some(sections) = created {
            commands.entity(entity).try_insert(sections);
        }
        if complete {
            commands.entity(entity).remove::<ChunkBuild>();
            if voxels.dirty_sections == 0 {
                commands
                    .entity(entity)
                    .remove::<(NeedsChunkBuild, EditedChunk)>();
            }
        }
        if applied_sections > 0
            && (uploaded >= UPLOAD_BUDGET_BYTES
                || meshes_applied >= UPLOAD_BUDGET_MESHES
                || started.elapsed() >= UPLOAD_BUDGET_TIME)
        {
            break;
        }
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
            commands
                .entity(entity)
                .try_insert(super::uploads::PendingMesh(mesh));
        }
        (Some(mesh), None) => {
            let transform = Transform::from_xyz(0.0, y, 0.0);
            *slot = Some(
                commands
                    .spawn((
                        bundle(mesh.clone()),
                        transform,
                        ChildOf(chunk),
                        super::uploads::PendingMesh(mesh),
                    ))
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
