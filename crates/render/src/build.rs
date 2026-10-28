//! Scheduling and application of asynchronous section meshes.

use super::{
    assets::VoxelAssets,
    environment::EnvironmentSettings,
    meshing::{ChunkNeighborhood, build_sections},
};
use crate::{
    RenderSource, VoxelViewer,
    sections::{PassGeometry, SectionFeed, SectionOp},
};
use bevy::{
    camera::{primitives::MeshAabb, visibility::RenderLayers},
    light::NotShadowCaster,
    mesh::Indices,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
use hollow_core::coordinates::{SECTION_HEIGHT, chunk_at};
use hollow_engine::streaming::AwaitingNeighbors;
use hollow_engine::{
    chunk::{ChunkVoxels, VoxelChunk},
    edits::EditedChunk,
    world::{ChunkIndex, WorldLayout},
};
use std::time::{Duration, Instant};

/// One contiguous section record array, sized to the active column geometry.
#[derive(Component)]
pub struct ChunkSections {
    pub sections: Box<[SectionEntities]>,
    pub missing_neighbors: u16,
    pub content_revision: u64,
    pub built_sections: u32,
    pub overhanging_sections: u32,
}
#[derive(Default)]
pub struct SectionEntities {
    pub translucent: Option<Entity>,
    pub passes: u8,
    pub connectivity: u64,
}
impl ChunkSections {
    fn new(count: usize) -> Self {
        Self {
            sections: (0..count).map(|_| SectionEntities::default()).collect(),
            missing_neighbors: 0,
            content_revision: 0,
            built_sections: 0,
            overhanging_sections: 0,
        }
    }
    pub fn mesh_count(&self) -> usize {
        self.sections
            .iter()
            .map(|s| s.passes.count_ones() as usize + usize::from(s.translucent.is_some()))
            .sum()
    }
}
/// GPU-ready geometry of one section, built off the main thread.
struct BuiltSection {
    index: usize,
    connectivity: u64,
    overhang: bool,
    bytes: usize,
    mesh_count: usize,
    terrain: Option<PassGeometry>,
    cutout: Option<PassGeometry>,
    models: Option<PassGeometry>,
    models_cutout: Option<PassGeometry>,
    /// Mesh sorted for the view at build time, its quads, and the view key it
    /// was sorted for (`None` without a view).
    translucent: Option<(
        Mesh,
        super::model_mesh::SortData,
        Option<super::sort::SortKey>,
    )>,
    shadow: Option<PassGeometry>,
}

struct BuildOutput {
    sections: std::vec::IntoIter<BuiltSection>,
    missing_neighbors: u16,
    #[cfg(feature = "dev")]
    meshing_elapsed: Duration,
    #[cfg(feature = "dev")]
    counts: (usize, usize),
}

/// Transient per build, so stored sparsely: starting and finishing a build never
/// moves the chunk's table row.
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct ChunkBuild {
    task: Task<BuildOutput>,
    ready: Option<BuildOutput>,
    content_revision: u64,
    revision: u64,
    shadows_enabled: bool,
}

/// Concurrent mesh tasks: enough to fill the compute pool without hoarding stale work.
fn max_pending_builds() -> usize {
    2 * AsyncComputeTaskPool::get().thread_num().max(1)
}

/// Mesh bytes handed to the GPU per frame, including player edits.
/// Spreads the upload of newly visible terrain over several frames.
const UPLOAD_BUDGET_BYTES: usize = 2 << 20;
/// Translucent mesh assets applied per frame; opaque geometry uses shared arenas.
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
pub struct NeedsChunkBuild;

type ChangedChunks<'w, 's, T> = Query<
    'w,
    's,
    (Entity, &'static ChunkVoxels<T>),
    (
        Changed<ChunkVoxels<T>>,
        Without<NeedsChunkBuild>,
        Without<AwaitingNeighbors>,
    ),
>;
type WaitingChunks<'w, 's, T> = Query<
    'w,
    's,
    (
        Entity,
        &'static VoxelChunk,
        &'static ChunkVoxels<T>,
        Has<EditedChunk>,
        Option<&'static ChunkSections>,
    ),
    (With<NeedsChunkBuild>, Without<ChunkBuild>),
>;

pub fn mark_changed_chunks<C: RenderSource>(
    mut commands: Commands,
    chunks: ChangedChunks<C::Cell>,
) {
    for (entity, voxels) in &chunks {
        if voxels.dirty_sections != 0 {
            commands.entity(entity).insert(NeedsChunkBuild);
        }
    }
}

pub(crate) fn update_shadow_geometry<C: RenderSource>(
    mut feed: ResMut<SectionFeed>,
    settings: Res<EnvironmentSettings>,
    mut previous: Local<Option<bool>>,
    mut chunks: Query<(&mut ChunkVoxels<C::Cell>, &mut ChunkSections)>,
) {
    let enabled = settings.shadows_enabled;
    if previous.replace(enabled) == Some(enabled) {
        return;
    }
    if !enabled {
        feed.ops.push(SectionOp::ClearPass(4));
    }
    for (mut voxels, mut sections) in &mut chunks {
        if enabled {
            voxels.mark_sections(sections.built_sections);
        } else {
            for section in &mut sections.sections {
                section.passes &= !(1 << 4);
            }
        }
    }
}

pub fn start_changed_builds<C: RenderSource>(
    mut commands: Commands,
    index: Res<ChunkIndex>,
    budget: Res<hollow_engine::tasks::VoxelTaskBudget>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    camera: Option<Single<&GlobalTransform, With<VoxelViewer>>>,
    builds: Query<Has<EditedChunk>, With<ChunkBuild>>,
    chunks: WaitingChunks<C::Cell>,
    all: Query<&ChunkVoxels<C::Cell>>,
    source: Res<C>,
    layout: Res<WorldLayout>,
    settings: Res<EnvironmentSettings>,
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
    let neighborhood = |position: IVec2, voxels: &ChunkVoxels<C::Cell>| {
        ChunkNeighborhood::new(voxels, source.clone(), layout.0, |offset| {
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
                        hollow_core::coordinates::chunk_distance_squared(position, center),
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
            neighborhood = neighborhood.reuse_connectivity(
                sections.sections.iter().map(|section| section.connectivity),
                sections.built_sections,
            );
        }
        commands.entity(entity).insert(spawn_chunk_build(
            &budget,
            if priority.0 {
                hollow_engine::tasks::WorkClass::Streaming
            } else {
                hollow_engine::tasks::WorkClass::EditedMesh
            },
            priority.1.0 as u32,
            neighborhood,
            voxels,
            eye,
            settings.shadows_enabled,
        ));
    }
}

fn spawn_chunk_build<C: RenderSource>(
    budget: &hollow_engine::tasks::VoxelTaskBudget,
    class: hollow_engine::tasks::WorkClass,
    distance: u32,
    neighborhood: ChunkNeighborhood<C>,
    voxels: &ChunkVoxels<C::Cell>,
    eye: Option<Vec3>,
    shadows_enabled: bool,
) -> ChunkBuild {
    let sections = voxels.dirty_sections;
    let revision = voxels.revision;
    let task = budget.spawn(class, distance, async move {
        #[cfg(feature = "dev")]
        let meshing_started = Instant::now();
        let sections = build_sections(&neighborhood, sections, shadows_enabled);
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
                terrain: section.terrain.map(PassGeometry::terrain),
                cutout: section.cutout.map(PassGeometry::terrain),
                models: section.models.map(PassGeometry::model),
                models_cutout: section.models_cutout.map(PassGeometry::model),
                translucent: section.translucent.map(|mut geometry| {
                    // First sort runs off the main thread; later sorts patch only GPU indices.
                    let origin = (neighborhood.origin
                        + IVec3::Y * (section.index as i32 * SECTION_HEIGHT))
                        .as_vec3();
                    let sort = geometry.take_sort_data();
                    let key = eye.map(|eye| {
                        geometry.indices = sort.indices(eye - origin);
                        super::sort::sort_key(eye, eye - origin)
                    });
                    (geometry.into_mesh(), sort, key)
                }),
                shadow: section.shadow.map(PassGeometry::terrain),
            })
            .collect::<Vec<_>>();
        for section in &mut sections {
            for geometry in [
                &section.terrain,
                &section.cutout,
                &section.models,
                &section.models_cutout,
                &section.shadow,
            ]
            .into_iter()
            .flatten()
            {
                section.overhang |= geometry.overhang();
                section.bytes += geometry.vertices.len() + geometry.indices.len();
            }
            if let Some((mesh, _, _)) = &section.translucent {
                section.overhang |= mesh.final_aabb.is_none_or(|bounds| {
                    bounds.min.min_element() < 0.0
                        || bounds.max.max_element() > SECTION_HEIGHT as f32
                });
                section.bytes += mesh_bytes(mesh);
                section.mesh_count = 1;
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
        shadows_enabled,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn poll_builds<C: RenderSource>(
    mut commands: Commands,
    assets: Option<Res<VoxelAssets>>,
    settings: Res<EnvironmentSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut feed: ResMut<SectionFeed>,
    layout: Res<WorldLayout>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    mut topology: Option<ResMut<super::visibility::VisibilityGraph>>,
    mut candidates: Local<Vec<((bool, i32, Entity), Entity)>>,
    mut chunks: Query<(
        Entity,
        &VoxelChunk,
        &mut ChunkVoxels<C::Cell>,
        &mut ChunkBuild,
        Option<&mut ChunkSections>,
        Has<EditedChunk>,
    )>,
    #[cfg(feature = "dev")] mut diagnostics: Option<ResMut<crate::diagnostics::VoxelDiagnostics>>,
) {
    let started = Instant::now();
    let (mut uploaded, mut meshes_applied, mut applied_sections) = (0usize, 0usize, 0usize);
    let center = viewer.map_or(IVec2::ZERO, |viewer| chunk_at(viewer.translation));
    candidates.clear();
    candidates.extend(chunks.iter().map(|(entity, chunk, _, _, _, edited)| {
        (
            (
                !edited,
                hollow_core::coordinates::chunk_distance_squared(chunk.position, center),
                entity,
            ),
            entity,
        )
    }));
    candidates.sort_unstable();
    for &(_, entity) in candidates.iter() {
        let Ok((_, chunk, mut voxels, mut build, sections, _)) = chunks.get_mut(entity) else {
            continue;
        };
        if build.revision != voxels.revision || build.shadows_enabled != settings.shadows_enabled {
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
            None => created.insert(ChunkSections::new(voxels.states.section_count())),
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
            if let Some(graph) = topology.as_deref_mut() {
                graph.invalidate_column(chunk.position, 1 << section.index);
            }
            entities.sections[section.index].connectivity = section.connectivity;
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
                let placement = (
                    Transform::from_xyz(0.0, (section.index as i32 * SECTION_HEIGHT) as f32, 0.0),
                    ChildOf(entity),
                );
                // New camera entities start in their likely layer, so an occluded
                // section needs no later insert. The invalidation above keeps the
                // camera conservative until visibility republishes this section.
                let camera = RenderLayers::layer(
                    if topology.as_deref().is_none_or(|graph| {
                        graph.likely_visible(chunk.position, section.index, section.overhang)
                    }) {
                        0
                    } else {
                        super::visibility::OCCLUDED_LAYER
                    },
                );
                let slots = &mut entities.sections[section.index];
                let geometry = [
                    section.terrain,
                    section.cutout,
                    section.models,
                    section.models_cutout,
                    section.shadow,
                ];
                slots.passes = geometry
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (i, g)| bits | (u8::from(g.is_some()) << i));
                feed.ops.push(SectionOp::Replace {
                    column: chunk.position,
                    section: section.index,
                    origin: Vec3::new(
                        (chunk.position.x * 16) as f32,
                        layout.min_y() as f32 + (section.index as i32 * SECTION_HEIGHT) as f32,
                        (chunk.position.y * 16) as f32,
                    ),
                    geometry,
                });
                let (translucent, sort) = section
                    .translucent
                    .map(|(mesh, sort, key)| (mesh, (sort, key)))
                    .unzip();
                update_section(
                    &mut commands,
                    &mut meshes,
                    &mut slots.translucent,
                    translucent,
                    sort,
                    |mesh, sort| {
                        let (sort, key) = sort.expect("translucent sort data");
                        (
                            Mesh3d(mesh),
                            MeshMaterial3d(assets.translucent.clone()),
                            NotShadowCaster,
                            camera.clone(),
                            placement.clone(),
                            sort,
                            super::sort::SortState::new(key),
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
                version.invalidate_column(chunk.position, u32::MAX);
            }
            entities.content_revision = build.content_revision;
        }
        if let Some(sections) = created {
            commands.entity(entity).try_insert(sections);
        }
        if complete {
            if voxels.dirty_sections == 0 {
                commands
                    .entity(entity)
                    .remove::<(ChunkBuild, NeedsChunkBuild, EditedChunk)>();
            } else {
                commands.entity(entity).remove::<ChunkBuild>();
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
    #[cfg(feature = "dev")]
    if let Some(diagnostics) = diagnostics.as_deref_mut() {
        diagnostics.mesh_apply_seconds += started.elapsed().as_secs_f64();
        diagnostics.uploaded_bytes += uploaded as u64;
        diagnostics.uploaded_meshes += meshes_applied as u64;
    }
}

type PendingSortData = (super::model_mesh::SortData, Option<super::sort::SortKey>);

/// Swaps a section's mesh, spawning or despawning its entity as it gains or loses faces.
/// A replacement waits in `PendingMesh` until uploaded; a new entity has no geometry
/// to keep, so it is spawned once with its final components.
fn update_section<B: Bundle>(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    slot: &mut Option<Entity>,
    mesh: Option<Mesh>,
    sort: Option<PendingSortData>,
    bundle: impl FnOnce(Handle<Mesh>, Option<PendingSortData>) -> B,
) {
    match (mesh, *slot) {
        (Some(mesh), Some(entity)) => {
            commands
                .entity(entity)
                .try_insert(super::uploads::PendingMesh {
                    mesh: meshes.add(mesh),
                    sort,
                });
        }
        (Some(mesh), None) => {
            // The bounds Bevy's `calculate_bounds` would otherwise insert a frame later.
            let aabb = mesh.get_aabb();
            let bundle = bundle(meshes.add(mesh), sort);
            *slot = Some(match aabb {
                Some(aabb) => commands.spawn((bundle, aabb)).id(),
                None => commands.spawn(bundle).id(),
            });
        }
        (None, Some(entity)) => {
            commands.entity(entity).try_despawn();
            *slot = None;
        }
        (None, None) => {}
    }
}
