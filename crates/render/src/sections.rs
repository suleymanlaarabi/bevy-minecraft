//! Shared opaque geometry, independent of Bevy's per-mesh instance machinery.
use crate::{VoxelViewer, assets::VoxelAssets};
use bevy::{
    asset::UntypedAssetId,
    camera::visibility::RenderLayers,
    core_pipeline::{
        core_3d::{AlphaMask3d, Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey},
        prepass::{Opaque3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey},
    },
    ecs::{
        query::ROQueryItem,
        system::{SystemParamItem, lifetimeless::SRes},
    },
    material::{
        MaterialProperties,
        key::{ErasedMaterialPipelineKey, ErasedMeshPipelineKey},
    },
    mesh::{MeshVertexBufferLayoutRef, MeshVertexBufferLayouts},
    pbr::*,
    platform::collections::HashMap,
    prelude::*,
    render::{
        ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems,
        erased_render_asset::ErasedRenderAssets,
        extract_component::ExtractComponentPlugin,
        material_bind_groups::MaterialBindGroupAllocators,
        mesh::allocator::MeshSlabs,
        render_phase::*,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::ExtractedView,
    },
    shader::ShaderDefVal,
};
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

pub(crate) struct PassGeometry {
    pub vertices: Vec<u8>,
    pub indices: Vec<u8>,
    count: u32,
    /// End of each face group in `indices` (`face_group` order), in indices.
    groups: [u32; 6],
    min: Vec3,
    max: Vec3,
}
/// Index groups put the three negative faces first, then the positive ones: the faces
/// a camera can see from one side of a section are then mostly contiguous.
fn face_group(vertex: [u32; 2]) -> usize {
    let direction = vertex[0] >> 15 & 7;
    ((direction & 1) * 3 + (direction >> 1)) as usize
}
impl PassGeometry {
    fn new(vertices: Vec<u8>, mut indices: Vec<u8>, count: usize, min: Vec3, max: Vec3) -> Self {
        indices.resize(indices.len().next_multiple_of(4), 0);
        let count = u32::try_from(count).expect("section index count");
        Self {
            vertices,
            indices,
            count,
            groups: [count; 6],
            min,
            max,
        }
    }
    pub fn terrain<I: crate::meshing::TerrainIndex + bytemuck::Pod>(
        geometry: crate::meshing::TerrainGeometry<I>,
    ) -> Self {
        let (min, max) = geometry
            .vertices
            .iter()
            .map(|&v| crate::meshing::TerrainGeometry::position(v))
            .fold((Vec3::MAX, Vec3::MIN), |(min, max), p| {
                (min.min(p), max.max(p))
            });
        // Counting sort of the triangles by face direction: the camera then draws
        // only the groups facing it. A triangle's vertices share one direction.
        let group = |triangle: &[I]| face_group(geometry.vertices[triangle[0].get() as usize]);
        let mut groups = [0u32; 6];
        for triangle in geometry.indices.chunks_exact(3) {
            groups[group(triangle)] += 3;
        }
        let mut next = [0usize; 6];
        let mut end = 0;
        for (group, next) in groups.iter_mut().zip(&mut next) {
            *next = end as usize;
            end += *group;
            *group = end;
        }
        let mut indices = geometry.indices.clone();
        for triangle in geometry.indices.chunks_exact(3) {
            let at = &mut next[group(triangle)];
            indices[*at..*at + 3].copy_from_slice(triangle);
            *at += 3;
        }
        let mut pass = Self::new(
            bytemuck::cast_slice(&geometry.vertices).to_vec(),
            bytemuck::cast_slice(&indices).to_vec(),
            indices.len(),
            min,
            max,
        );
        pass.groups = groups;
        pass
    }
    pub fn model(geometry: crate::model_mesh::ModelGeometry) -> Self {
        Self::new(
            bytemuck::cast_slice(&geometry.vertices).to_vec(),
            bytemuck::cast_slice(&geometry.indices).to_vec(),
            geometry.indices.len(),
            geometry.min,
            geometry.max,
        )
    }
    pub fn overhang(&self) -> bool {
        self.min.min_element() < 0.0 || self.max.max_element() > 16.0
    }
}
pub(crate) enum SectionOp {
    Replace {
        column: IVec2,
        section: usize,
        origin: Vec3,
        geometry: [Option<PassGeometry>; 5],
        /// Sunlight reaches the section or its border. Buried cutouts and models
        /// stay out of the shadow maps, like buried terrain (`SectionMeshes::shadow`).
        casts_shadow: bool,
    },
    Visibility {
        column: IVec2,
        mask: u32,
    },
    RemoveColumn(IVec2),
    ClearPass(usize),
    Clear,
}
#[derive(Resource, Default)]
pub(crate) struct SectionFeed {
    pub ops: Vec<SectionOp>,
    pub conservative: bool,
}
#[derive(Resource)]
struct SectionIds([MainEntity; RANGES]);

pub(crate) struct SectionsPlugin;
impl Plugin for SectionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SectionFeed>();
        let ids = std::array::from_fn(|_| MainEntity::from(app.world_mut().spawn_empty().id()));
        app.insert_resource(SectionIds(ids));
        app.add_observer(
            |_: On<hollow_engine::session::SessionClosed>, mut feed: ResMut<SectionFeed>| {
                feed.ops.clear();
                feed.ops.push(SectionOp::Clear);
                feed.conservative = true;
            },
        );
        app.add_plugins(ExtractComponentPlugin::<VoxelViewer>::default());
        bevy::shader::load_shader_library!(app, "shaders/section_transform.wesl");
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .init_resource::<SectionFeed>()
            .init_resource::<SpecializedMeshPipelines<SectionSpecializer>>()
            .add_systems(ExtractSchedule, extract_sections)
            .add_systems(
                Render,
                (prepare_sections, queue_sections)
                    .chain()
                    .in_set(RenderSystems::QueueMeshes),
            );
        register_draws::<0>(render);
        register_draws::<1>(render);
        register_draws::<2>(render);
        register_draws::<3>(render);
        register_draws::<4>(render);
        register_draws::<5>(render);
    }
    fn finish(&self, app: &mut App) {
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        let gpu = SectionGpu::new(render.world_mut());
        render.insert_resource(gpu);
    }
}
fn register_draws<const P: usize>(app: &mut SubApp) {
    app.add_render_command::<Opaque3d, MainDraw<P>>()
        .add_render_command::<AlphaMask3d, MainDraw<P>>()
        .add_render_command::<Opaque3dPrepass, DepthDraw<P>>()
        .add_render_command::<Shadow, DepthDraw<P>>();
}
fn extract_sections(
    mut main: ResMut<MainWorld>,
    mut feed: ResMut<SectionFeed>,
    mut commands: Commands,
) {
    let mut source = main.resource_mut::<SectionFeed>();
    std::mem::swap(&mut feed.ops, &mut source.ops);
    feed.conservative = source.conservative;
    let ids = main.resource::<SectionIds>();
    commands.insert_resource(SectionIds(ids.0));
    let materials = main.get_resource::<VoxelAssets>().map(|a| {
        [
            a.terrain.id().untyped(),
            a.cutout.id().untyped(),
            a.models.id().untyped(),
            a.models_cutout.id().untyped(),
            a.terrain.id().untyped(),
            a.terrain.id().untyped(),
        ]
    });
    commands.insert_resource(SectionMaterials(materials));
}
#[derive(Resource)]
struct SectionMaterials(Option<[UntypedAssetId; RANGES]>);

/// Sparse columns own section slots and conservative bounds for hierarchical culling.
struct Column {
    slots: [u32; 32],
    occupied: u32,
    mask: u32,
    min: Vec3,
    max: Vec3,
}
impl Default for Column {
    fn default() -> Self {
        Self {
            slots: [0; 32],
            occupied: 0,
            mask: u32::MAX,
            min: Vec3::INFINITY,
            max: Vec3::NEG_INFINITY,
        }
    }
}
struct Slot {
    passes: [Option<Allocation>; 5],
}
/// Per-slot culling inputs, dense and parallel to `SectionGpu::slots`: the per-view
/// loop streams 28 bytes per slot and reads allocations only for drawn sections.
#[derive(Clone, Copy, Default)]
struct Cull {
    center: Vec3,
    extent: Vec3,
    /// Bit per pass holding geometry; zero for a free slot.
    passes: u8,
    /// `passes` drawn into shadow maps.
    casters: u8,
}
/// Passes drawn into shadow maps from sections that sunlight never reaches: only the
/// shadow hull, which is built for sunlit sections alone.
const BURIED_CASTERS: u8 = 1 << 4;
/// Draw ranges of one view. Depth passes record what they drew: with occlusion
/// culling, Bevy renders the same phase again in its late prepass and late shadow
/// passes, where these CPU-culled sections would only repeat identical depth.
struct ViewDraws {
    ranges: [Range<usize>; RANGES],
    depth_drawn: AtomicU8,
}
struct Allocation {
    vertices: Range<u64>,
    indices: Range<u64>,
    /// Indirect arguments, fixed while the allocation lives.
    draw: [u32; 5],
    /// `PassGeometry::groups`.
    groups: [u32; 6],
}
/// Draw ranges of a view: the five geometry passes, then the camera's far terrain.
const RANGES: usize = 6;
/// Terrain (pass 0 geometry and material) past `PREPASS_BUCKETS`, drawn by the main
/// pass alone with depth writes.
const FAR_TERRAIN: usize = 5;
const fn range_pass(range: usize) -> usize {
    if range == FAR_TERRAIN { 0 } else { range }
}
/// The depth prepass keeps the costly shadowed shading of near terrain to one
/// fragment per pixel. Past the shadow cascades (128 blocks plus a section) shading
/// is a texture and fog, most of a long view's triangles are there and sections
/// already draw front to back: rasterizing them twice costs more than it saves.
const PREPASS_BUCKETS: usize = 20;
/// Sections with at most this many indices draw whole: a second draw would cost
/// more than the back faces it removes.
const WHOLE_DRAW_INDICES: u32 = 384;
/// Hidden indices drawn between two visible face groups rather than splitting a draw.
const MERGE_GAP: u32 = 96;
const ALL_FACES: u8 = 0b111111;

/// Calls `emit(first index, index count)` for each run of non-empty `faces` groups.
#[inline]
fn face_runs(groups: &[u32; 6], faces: u8, mut emit: impl FnMut(u32, u32)) {
    let (mut start, mut end, mut previous) = (0, 0, 0);
    for (group, &next) in groups.iter().enumerate() {
        if faces & 1 << group != 0 && next > previous {
            if end == start {
                start = previous;
            } else if previous - end > MERGE_GAP {
                emit(start, end - start);
                start = previous;
            }
            end = next;
        }
        previous = next;
    }
    if end != start {
        emit(start, end - start);
    }
}
/// Bytes per index and vertex of each pass (u16 terrain and shadow indices,
/// 16-byte model vertices).
const INDEX_BYTES: [u64; 5] = [2, 4, 4, 4, 2];
const VERTEX_BYTES: [u64; 5] = [8, 8, 16, 16, 8];
/// Front-to-back draw order in 8-block distance buckets.
const DISTANCE_BUCKETS: usize = 256;
const BUCKETS_PER_BLOCK: f32 = 1.0 / 8.0;

/// Culling inputs of one view; unchanged views and sections keep last frame's draws.
#[derive(Clone, Copy, PartialEq)]
struct ViewCull {
    entity: Entity,
    clip: Mat4,
    eye: Vec3,
    enabled: u8,
    camera: bool,
}
fn section_buffer(device: &RenderDevice, label: &str, size: u64, usage: BufferUsages) -> Buffer {
    device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn section_bind_group(
    device: &RenderDevice,
    layout: &BindGroupLayoutDescriptor,
    origins: &Buffer,
) -> BindGroup {
    device.create_bind_group(
        "section origins",
        &device.create_bind_group_layout(layout.label.as_ref(), &layout.entries),
        &[BindGroupEntry {
            binding: 32,
            resource: origins.as_entire_binding(),
        }],
    )
}
struct ArenaBuffer {
    buffer: Buffer,
    end: u64,
    free: Vec<Range<u64>>,
}
impl ArenaBuffer {
    fn new(device: &RenderDevice, usage: BufferUsages) -> Self {
        let capacity = 4096;
        Self {
            buffer: section_buffer(
                &device,
                "section arena",
                capacity,
                usage | BufferUsages::COPY_SRC,
            ),
            end: 0,
            free: Vec::new(),
        }
    }
    fn allocate(&mut self, size: u64) -> Range<u64> {
        if let Some(i) = self.free.iter().position(|r| r.end - r.start >= size) {
            let start = self.free[i].start;
            self.free[i].start += size;
            if self.free[i].is_empty() {
                self.free.remove(i);
            }
            return start..start + size;
        }
        let start = self.end;
        self.end += size;
        start..self.end
    }
    fn release(&mut self, mut range: Range<u64>) {
        let mut i = self.free.partition_point(|r| r.start < range.start);
        if i > 0 && self.free[i - 1].end == range.start {
            i -= 1;
            range.start = self.free.remove(i).start;
        }
        if i < self.free.len() && range.end == self.free[i].start {
            range.end = self.free.remove(i).end;
        }
        self.free.insert(i, range);
        while self.free.last().is_some_and(|r| r.end == self.end) {
            self.end = self.free.pop().unwrap().start;
        }
    }
    fn grow(&mut self, device: &RenderDevice, encoder: &mut CommandEncoder) {
        if self.end <= self.buffer.size() {
            return;
        }
        let capacity = self.end.next_power_of_two();
        let buffer = section_buffer(&device, "section arena", capacity, self.buffer.usage());
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &buffer, 0, self.buffer.size());
        self.buffer = buffer;
    }
}
struct Arena {
    vertices: ArenaBuffer,
    indices: ArenaBuffer,
}
#[derive(Resource)]
struct SectionGpu {
    arenas: [Arena; 5],
    columns: HashMap<IVec2, Column>,
    slots: Vec<Option<Slot>>,
    cull: Vec<Cull>,
    free: Vec<usize>,
    origins: Buffer,
    /// CPU copy of `origins`; only the slots replaced since the last upload are written.
    origin_data: Vec<[f32; 4]>,
    origins_dirty: Option<Range<usize>>,
    /// Views the current `args` and `ranges` were built for.
    views: Vec<ViewCull>,
    conservative: bool,
    bind_group: BindGroup,
    layout: BindGroupLayoutDescriptor,
    layouts: [MeshVertexBufferLayoutRef; 2],
    indirect: Buffer,
    args: Vec<[u32; 5]>,
    ranges: HashMap<Entity, ViewDraws>,
    indirect_first_instance: bool,
}
impl SectionGpu {
    fn new(world: &mut World) -> Self {
        let layouts = {
            let mut layouts = world.resource_mut::<MeshVertexBufferLayouts>();
            [
                Mesh::new(PrimitiveTopology::TriangleList, default())
                    .with_inserted_attribute(
                        crate::meshing::ATTRIBUTE_VOXEL,
                        Vec::<[u32; 2]>::new(),
                    )
                    .get_mesh_vertex_buffer_layout(&mut layouts),
                Mesh::new(PrimitiveTopology::TriangleList, default())
                    .with_inserted_attribute(
                        crate::model_mesh::ATTRIBUTE_MODEL,
                        Vec::<[u32; 4]>::new(),
                    )
                    .get_mesh_vertex_buffer_layout(&mut layouts),
            ]
        };
        let device = world.resource::<RenderDevice>();
        let layout = BindGroupLayoutDescriptor::new(
            "section origins",
            &[BindGroupLayoutEntry {
                binding: 32,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        );
        let origins = section_buffer(&device, "section origins", 16, BufferUsages::STORAGE);
        let bind_group = section_bind_group(device, &layout, &origins);
        Self {
            arenas: std::array::from_fn(|_| Arena {
                vertices: ArenaBuffer::new(device, BufferUsages::VERTEX),
                indices: ArenaBuffer::new(device, BufferUsages::INDEX),
            }),
            columns: default(),
            slots: Vec::new(),
            cull: Vec::new(),
            free: Vec::new(),
            origins,
            origin_data: Vec::new(),
            origins_dirty: None,
            views: Vec::new(),
            conservative: false,
            bind_group,
            layout,
            layouts,
            indirect: section_buffer(&device, "section draws", 20, BufferUsages::INDIRECT),
            args: Vec::new(),
            ranges: default(),
            indirect_first_instance: device
                .features()
                .contains(WgpuFeatures::INDIRECT_FIRST_INSTANCE),
        }
    }
    fn release(&mut self, slot: usize, pass: usize) {
        if let Some(a) = self.slots[slot]
            .as_mut()
            .and_then(|s| s.passes[pass].take())
        {
            self.arenas[pass].vertices.release(a.vertices);
            self.arenas[pass].indices.release(a.indices);
            self.cull[slot].passes &= !(1 << pass);
            self.cull[slot].casters &= !(1 << pass);
        }
    }
    fn free_slot(&mut self, slot: usize) {
        for pass in 0..5 {
            self.release(slot, pass);
        }
        self.slots[slot] = None;
        self.cull[slot] = Cull::default();
        self.free.push(slot);
    }
    fn remove(&mut self, column: IVec2) {
        if let Some(column) = self.columns.remove(&column) {
            let mut sections = column.occupied;
            while sections != 0 {
                self.free_slot(column.slots[sections.trailing_zeros() as usize] as usize);
                sections &= sections - 1;
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn prepare_sections(
    mut gpu: ResMut<SectionGpu>,
    mut feed: ResMut<SectionFeed>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    views: Query<(
        Entity,
        &ExtractedView,
        Option<&VoxelViewer>,
        Option<&LightEntity>,
    )>,
    lights: Query<Option<&RenderLayers>>,
    mut current: Local<Vec<ViewCull>>,
    mut workers: Local<Vec<CullWorker>>,
) {
    let gpu = &mut *gpu;
    let changed = !feed.ops.is_empty() || gpu.conservative != feed.conservative;
    gpu.conservative = feed.conservative;
    let mut uploads = Vec::new();
    for op in feed.ops.drain(..) {
        match op {
            SectionOp::Clear => {
                gpu.columns.clear();
                gpu.slots.clear();
                gpu.cull.clear();
                gpu.origin_data.clear();
                gpu.origins_dirty = None;
                gpu.free.clear();
                for arena in &mut gpu.arenas {
                    for buffer in [&mut arena.vertices, &mut arena.indices] {
                        buffer.end = 0;
                        buffer.free.clear();
                    }
                }
                uploads.clear();
            }
            SectionOp::RemoveColumn(column) => gpu.remove(column),
            SectionOp::ClearPass(pass) => {
                for slot in 0..gpu.slots.len() {
                    gpu.release(slot, pass);
                }
            }
            SectionOp::Visibility { column, mask } => {
                gpu.columns.entry(column).or_default().mask = mask;
            }
            SectionOp::Replace {
                column,
                section,
                origin,
                geometry,
                casts_shadow,
            } => {
                let c = gpu.columns.entry(column).or_default();
                let bit = 1 << section;
                if geometry.iter().all(Option::is_none) {
                    if c.occupied & bit != 0 {
                        c.occupied &= !bit;
                        let slot = c.slots[section] as usize;
                        gpu.free_slot(slot);
                    }
                    continue;
                }
                let slot = if c.occupied & bit == 0 {
                    let slot = gpu.free.pop().unwrap_or_else(|| {
                        gpu.slots.push(None);
                        gpu.cull.push(Cull::default());
                        gpu.origin_data.push([0.0; 4]);
                        gpu.slots.len() - 1
                    });
                    c.slots[section] = u32::try_from(slot).expect("section slot");
                    c.occupied |= bit;
                    slot
                } else {
                    c.slots[section] as usize
                };
                for pass in 0..5 {
                    gpu.release(slot, pass);
                }
                let mut record = Slot {
                    passes: std::array::from_fn(|_| None),
                };
                let (mut min, mut max, mut passes) = (Vec3::MAX, Vec3::MIN, 0u8);
                for (pass, geometry) in geometry.into_iter().enumerate() {
                    if let Some(g) = geometry {
                        min = min.min(g.min + origin);
                        max = max.max(g.max + origin);
                        let vertices = gpu.arenas[pass].vertices.allocate(g.vertices.len() as u64);
                        let indices = gpu.arenas[pass].indices.allocate(g.indices.len() as u64);
                        let draw = [
                            g.count,
                            1,
                            u32::try_from(indices.start / INDEX_BYTES[pass])
                                .expect("arena index offset"),
                            i32::try_from(vertices.start / VERTEX_BYTES[pass])
                                .expect("arena vertex offset") as u32,
                            u32::try_from(slot).expect("section slot"),
                        ];
                        let groups = g.groups;
                        uploads.push((slot, pass, vertices.start, indices.start, g));
                        record.passes[pass] = Some(Allocation {
                            vertices,
                            indices,
                            draw,
                            groups,
                        });
                        passes |= 1 << pass;
                    }
                }
                // Bounds only expand until the column is retired. Stale space is
                // conservative after replacement and avoids rescanning its sections.
                let c = gpu.columns.get_mut(&column).unwrap();
                c.min = c.min.min(min);
                c.max = c.max.max(max);
                gpu.slots[slot] = Some(record);
                gpu.cull[slot] = Cull {
                    center: (min + max) * 0.5,
                    extent: (max - min) * 0.5,
                    passes,
                    casters: if casts_shadow {
                        passes
                    } else {
                        passes & BURIED_CASTERS
                    },
                };
                gpu.origin_data[slot] = origin.extend(0.0).to_array();
                gpu.origins_dirty = Some(
                    gpu.origins_dirty
                        .take()
                        .map_or(slot..slot + 1, |r| r.start.min(slot)..r.end.max(slot + 1)),
                );
            }
        }
    }
    // Later operations can remove or replace geometry before extraction finishes.
    uploads.retain(|(slot, pass, v, i, g)| {
        gpu.slots
            .get(*slot)
            .and_then(Option::as_ref)
            .and_then(|s| s.passes[*pass].as_ref())
            .is_some_and(|a| {
                a.vertices == (*v..*v + g.vertices.len() as u64)
                    && a.indices == (*i..*i + g.indices.len() as u64)
            })
    });
    // Submit growth copies before queue writes: otherwise old bytes overwrite uploads.
    if !uploads.is_empty() {
        if gpu.arenas.iter().any(|a| {
            a.vertices.end > a.vertices.buffer.size() || a.indices.end > a.indices.buffer.size()
        }) {
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
                label: Some("grow section arenas"),
            });
            for arena in &mut gpu.arenas {
                arena.vertices.grow(&device, &mut encoder);
                arena.indices.grow(&device, &mut encoder);
            }
            queue.submit([encoder.finish()]);
        }
        for (_, p, v, i, g) in uploads {
            queue.write_buffer(&gpu.arenas[p].vertices.buffer, v, &g.vertices);
            queue.write_buffer(&gpu.arenas[p].indices.buffer, i, &g.indices);
        }
    }
    if gpu.slots.len() as u64 > gpu.origins.size() / 16 {
        gpu.origins = section_buffer(
            &device,
            "section origins",
            (gpu.slots.len().next_power_of_two() * 16) as u64,
            BufferUsages::STORAGE,
        );
        gpu.bind_group = section_bind_group(&device, &gpu.layout, &gpu.origins);
        gpu.origins_dirty = Some(0..gpu.slots.len());
    }
    if let Some(range) = gpu.origins_dirty.take()
        && !range.is_empty()
    {
        queue.write_buffer(
            &gpu.origins,
            (range.start * 16) as u64,
            bytemuck::cast_slice(&gpu.origin_data[range]),
        );
    }
    for draws in gpu.ranges.values_mut() {
        *draws.depth_drawn.get_mut() = 0;
    }
    current.clear();
    for (entity, view, camera, light) in &views {
        let shadow_layers = match light {
            Some(LightEntity::Directional { light_entity, .. }) => Some(
                lights
                    .get(*light_entity)
                    .ok()
                    .flatten()
                    .cloned()
                    .unwrap_or_default(),
            ),
            Some(_) => continue,
            None if camera.is_some() => None,
            _ => continue,
        };
        let enabled = shadow_layers.as_ref().map_or(0b01111, |layers| {
            let models = layers.intersects(&RenderLayers::from_layers(&[
                0,
                crate::visibility::OCCLUDED_LAYER,
            ]));
            let terrain = layers.intersects(&RenderLayers::layer(crate::SHADOW_LAYER));
            u8::from(models) * 0b01110 | u8::from(terrain) << 4
        });
        current.push(ViewCull {
            entity,
            clip: view.clip_from_world.unwrap_or_else(|| {
                view.clip_from_view * view.world_from_view.to_matrix().inverse()
            }),
            eye: view.world_from_view.translation(),
            enabled,
            camera: shadow_layers.is_none(),
        });
    }
    // Still views over unchanged sections keep last frame's draws and GPU arguments.
    if !changed && *current == gpu.views {
        return;
    }
    let mut arg_end = 0;
    gpu.ranges.clear();
    let columns: Vec<&Column> = gpu.columns.values().collect();
    let pool = bevy::tasks::ComputeTaskPool::get();
    let threads = pool.thread_num().clamp(1, 8);
    for view in current.iter() {
        let rows = view
            .clip
            .transpose()
            .to_cols_array_2d()
            .map(Vec4::from_array);
        // Shadow views keep every caster between the light and their near plane, but
        // drop sections past their far plane: depth there is clipped or clamped to the
        // cleared far value, so they can never change the map.
        let depth = if view.camera {
            rows[3] - rows[2]
        } else {
            rows[2]
        };
        let planes: [Plane; 5] = [
            rows[3] + rows[0],
            rows[3] - rows[0],
            rows[3] + rows[1],
            rows[3] - rows[1],
            depth,
        ]
        .map(|p| (p.truncate(), p.truncate().abs(), p.w));
        let require_visible = view.camera && !feed.conservative;
        // Every camera move rebuilds the draws: columns are culled in parallel, each
        // worker counting and collecting its own draws.
        let chunk = columns.len().div_ceil(threads).max(128);
        let jobs = columns.len().div_ceil(chunk);
        if workers.len() < jobs {
            workers.resize_with(jobs, default);
        }
        let workers = &mut workers[..jobs];
        let (cull, slots) = (&gpu.cull[..], &gpu.slots[..]);
        if let [worker] = &mut *workers {
            cull_columns(
                &columns,
                cull,
                slots,
                view,
                &planes,
                require_visible,
                worker,
            );
        } else {
            let planes = &planes;
            pool.scope(|scope| {
                for (columns, worker) in columns.chunks(chunk).zip(workers.iter_mut()) {
                    scope.spawn(async move {
                        cull_columns(columns, cull, slots, view, planes, require_visible, worker);
                    });
                }
            });
        }
        // Prefix sums place each range directly into the final indirect buffer,
        // nearest buckets first. Shadow views use just one unsorted bucket.
        let buckets = if view.camera { DISTANCE_BUCKETS } else { 1 };
        let ranges = std::array::from_fn(|p| {
            let start = arg_end;
            for bucket in 0..buckets {
                for worker in workers.iter_mut() {
                    let count = &mut worker.counts[p][bucket];
                    let next = arg_end + *count as usize;
                    *count = arg_end as u32;
                    arg_end = next;
                }
            }
            start..arg_end
        });
        if gpu.args.len() < arg_end {
            gpu.args.resize(arg_end, [0; 5]);
        }
        for CullWorker { counts, draws } in workers.iter_mut() {
            for &(range, bucket, draw) in draws.iter() {
                let at = &mut counts[usize::from(range)][usize::from(bucket)];
                gpu.args[*at as usize] = draw;
                *at += 1;
            }
        }
        gpu.ranges.insert(
            view.entity,
            ViewDraws {
                ranges,
                depth_drawn: AtomicU8::new(0),
            },
        );
    }
    gpu.args.truncate(arg_end);
    gpu.views.clone_from(&current);
    if gpu.args.len() as u64 > gpu.indirect.size() / 20 {
        gpu.indirect = section_buffer(
            &device,
            "section draws",
            (gpu.args.len().next_power_of_two() * 20) as u64,
            BufferUsages::INDIRECT,
        );
    }
    if !gpu.args.is_empty() && gpu.indirect_first_instance {
        queue.write_buffer(&gpu.indirect, 0, bytemuck::cast_slice(&gpu.args));
    }
}

/// Frustum plane: normal, absolute normal and offset. Scalar on purpose: the
/// culling loops run per section every frame the camera moves.
type Plane = (Vec3, Vec3, f32);

/// Reject outside bounds and pass only intersecting planes down the hierarchy.
#[inline]
fn cull_planes(center: Vec3, extent: Vec3, planes: &[Plane; 5], mut mask: u8) -> Option<u8> {
    let mut remaining = mask;
    while remaining != 0 {
        let plane = remaining.trailing_zeros() as usize;
        let (n, a, w) = &planes[plane];
        let distance = n.x * center.x + n.y * center.y + n.z * center.z + w;
        let radius = a.x * extent.x + a.y * extent.y + a.z * extent.z;
        if distance + radius < 0.0 {
            return None;
        }
        if distance >= radius {
            mask &= !(1 << plane);
        }
        remaining &= remaining - 1;
    }
    Some(mask)
}

/// Draws and per-bucket counts one culling task collects for a view.
struct CullWorker {
    counts: Box<[[u32; DISTANCE_BUCKETS]; RANGES]>,
    draws: Vec<(u8, u8, [u32; 5])>,
}
impl Default for CullWorker {
    fn default() -> Self {
        Self {
            counts: Box::new([[0; DISTANCE_BUCKETS]; RANGES]),
            draws: Vec::new(),
        }
    }
}

fn cull_columns(
    columns: &[&Column],
    cull: &[Cull],
    slots: &[Option<Slot>],
    view: &ViewCull,
    planes: &[Plane; 5],
    require_visible: bool,
    worker: &mut CullWorker,
) {
    let CullWorker { counts, draws } = worker;
    for range in counts.iter_mut() {
        range.fill(0);
    }
    draws.clear();
    let eye = view.eye;
    for column in columns {
        let mut sections = column.occupied
            & if require_visible {
                column.mask
            } else {
                u32::MAX
            };
        if sections == 0 {
            continue;
        }
        let (min, max) = (column.min, column.max);
        let Some(mask) = cull_planes(
            Vec3::new(
                (min.x + max.x) * 0.5,
                (min.y + max.y) * 0.5,
                (min.z + max.z) * 0.5,
            ),
            Vec3::new(
                (max.x - min.x) * 0.5,
                (max.y - min.y) * 0.5,
                (max.z - min.z) * 0.5,
            ),
            planes,
            0b11111,
        ) else {
            continue;
        };
        while sections != 0 {
            let slot = column.slots[sections.trailing_zeros() as usize] as usize;
            sections &= sections - 1;
            let c = &cull[slot];
            let passes = (if view.camera { c.passes } else { c.casters }) & view.enabled;
            if passes == 0 || (mask != 0 && cull_planes(c.center, c.extent, planes, mask).is_none())
            {
                continue;
            }
            // A face can only be seen from the outer side of its plane, which
            // lies within the section's bounds: skip the groups facing away.
            let (bucket, faces) = if view.camera {
                let (center, extent) = (c.center, c.extent);
                let (x, y, z) = (center.x - eye.x, center.y - eye.y, center.z - eye.z);
                (
                    (((x * x + y * y + z * z).sqrt() * BUCKETS_PER_BLOCK) as usize)
                        .min(DISTANCE_BUCKETS - 1),
                    u8::from(eye.x < center.x + extent.x)
                        | u8::from(eye.y < center.y + extent.y) << 1
                        | u8::from(eye.z < center.z + extent.z) << 2
                        | u8::from(eye.x > center.x - extent.x) << 3
                        | u8::from(eye.y > center.y - extent.y) << 4
                        | u8::from(eye.z > center.z - extent.z) << 5,
                )
            } else {
                (0, ALL_FACES)
            };
            let allocations = &slots[slot].as_ref().unwrap().passes;
            let mut bits = passes;
            while bits != 0 {
                let pass = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let a = allocations[pass].as_ref().unwrap();
                let range = if view.camera && pass == 0 && bucket >= PREPASS_BUCKETS {
                    FAR_TERRAIN
                } else {
                    pass
                };
                // Only greedy cube faces are grouped; shadow views draw both sides.
                if !view.camera || pass > 1 || a.draw[0] <= WHOLE_DRAW_INDICES {
                    counts[range][bucket] += 1;
                    draws.push((range as u8, bucket as u8, a.draw));
                    continue;
                }
                face_runs(&a.groups, faces, |first, count| {
                    counts[range][bucket] += 1;
                    let mut draw = a.draw;
                    draw[0] = count;
                    draw[2] += first;
                    draws.push((range as u8, bucket as u8, draw));
                });
            }
        }
    }
}

struct SectionSpecializer {
    mesh: MaterialPipeline,
    prepass: PrepassPipeline,
    properties: Arc<MaterialProperties>,
    layout: BindGroupLayoutDescriptor,
}
impl SpecializedMeshPipeline for SectionSpecializer {
    /// Depth pass, then whether a main pass reuses the prepass depth unchanged.
    type Key = (bool, bool, ErasedMaterialPipelineKey);
    fn specialize(
        &self,
        (depth, reuse_depth, key): Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut d = if depth {
            PrepassPipelineSpecializer {
                pipeline: self.prepass.clone(),
                properties: self.properties.clone(),
            }
            .specialize(key, layout)?
        } else {
            let mut d = self
                .mesh
                .mesh_pipeline
                .specialize(key.mesh_key.downcast(), layout)?;
            d.vertex
                .shader_defs
                .push(ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3));
            if let Some(f) = &mut d.fragment {
                f.shader_defs
                    .push(ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3));
            }
            if let Some(s) = self.properties.get_shader(MaterialVertexShader) {
                d.vertex.shader = s;
            }
            if let Some(s) = self.properties.get_shader(MaterialFragmentShader) {
                d.fragment.as_mut().unwrap().shader = s;
            }
            d.layout
                .insert(3, self.properties.material_layout.clone().unwrap());
            if let Some(specialize) = self.properties.user_specialize {
                specialize(&self.mesh, &mut d, layout, key)?;
            }
            // The depth prepass already wrote this exact (`@invariant`) depth.
            if reuse_depth && let Some(depth_stencil) = &mut d.depth_stencil {
                depth_stencil.depth_write_enabled = Some(false);
            }
            d
        };
        d.layout[2] = self.layout.clone();
        d.layout[3] = self.properties.material_layout.clone().unwrap();
        d.vertex.shader_defs.push("VOXEL_SECTIONS".into());
        if let Some(f) = &mut d.fragment {
            f.shader_defs.push("VOXEL_SECTIONS".into());
        }
        Ok(d)
    }
}
/// Section draws are retained `NonMesh` phase items: one is replaced only when its
/// pipeline or presence changes instead of being removed and re-added every frame.
#[allow(clippy::too_many_arguments)]
fn queue_sections(
    gpu: Res<SectionGpu>,
    ids: Res<SectionIds>,
    materials: Res<SectionMaterials>,
    prepared: Res<ErasedRenderAssets<PreparedMaterial>>,
    mesh: Res<MeshPipeline>,
    prepass: Res<PrepassPipeline>,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedMeshPipelines<SectionSpecializer>>,
    views: Query<(Entity, &ExtractedView, Option<&LightEntity>)>,
    (keys, depth_keys, light_keys): (
        Res<ViewKeyCache>,
        Res<ViewKeyPrepassCache>,
        Res<LightKeyCache>,
    ),
    mut opaque: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    mut alpha: ResMut<ViewBinnedRenderPhases<AlphaMask3d>>,
    mut depth: ResMut<ViewBinnedRenderPhases<Opaque3dPrepass>>,
    mut shadows: ResMut<ViewBinnedRenderPhases<Shadow>>,
    (od, ad, dd, sd): (
        Res<DrawFunctions<Opaque3d>>,
        Res<DrawFunctions<AlphaMask3d>>,
        Res<DrawFunctions<Opaque3dPrepass>>,
        Res<DrawFunctions<Shadow>>,
    ),
) {
    let Some(materials) = &materials.0 else {
        return;
    };
    let (od, ad, dd, sd) = (od.read(), ad.read(), dd.read(), sd.read());
    let main_draws = [
        od.id::<MainDraw<0>>(),
        ad.id::<MainDraw<1>>(),
        od.id::<MainDraw<2>>(),
        ad.id::<MainDraw<3>>(),
        od.id::<MainDraw<4>>(),
        od.id::<MainDraw<5>>(),
    ];
    let shadow_draws = [
        sd.id::<DepthDraw<0>>(),
        sd.id::<DepthDraw<1>>(),
        sd.id::<DepthDraw<2>>(),
        sd.id::<DepthDraw<3>>(),
        sd.id::<DepthDraw<4>>(),
    ];
    let depth_draw = dd.id::<DepthDraw<0>>();
    let topology = MeshPipelineKey::from_primitive_topology_and_strip_index(
        PrimitiveTopology::TriangleList,
        None,
    );
    let asset_id = AssetId::<Mesh>::default().untyped();
    let mut specializer = None;
    for (p, &material_id) in materials.iter().enumerate() {
        let Some(material) = prepared.get(material_id) else {
            continue;
        };
        let specializer = specializer.get_or_insert_with(|| SectionSpecializer {
            mesh: MaterialPipeline {
                mesh_pipeline: mesh.clone(),
            },
            prepass: prepass.clone(),
            properties: material.properties.clone(),
            layout: gpu.layout.clone(),
        });
        specializer.properties = material.properties.clone();
        let layout = &gpu.layouts[usize::from(p == 2 || p == 3)];
        let key_for = |view_key: MeshPipelineKey, depth: bool, reuse_depth: bool| {
            let mut key = view_key | topology;
            if !depth {
                key |= material
                    .properties
                    .mesh_pipeline_key_bits
                    .downcast::<MeshPipelineKey>();
            }
            key |= alpha_mode_pipeline_key(
                material.properties.alpha_mode,
                &Msaa::from_samples(view_key.msaa_samples()),
            );
            if depth {
                key |= MeshPipelineKey::PREPASS_READS_MATERIAL;
            }
            (
                depth,
                reuse_depth,
                ErasedMaterialPipelineKey {
                    mesh_key: ErasedMeshPipelineKey::new(key),
                    material_key: material.properties.material_key.clone(),
                    type_id: material_id.type_id(),
                },
            )
        };
        let item = |entity| (entity, ids.0[p]);
        for (entity, view, light) in &views {
            let retained = view.retained_view_entity;
            let active = gpu
                .ranges
                .get(&entity)
                .is_some_and(|draws| !draws.ranges[p].is_empty());
            let mut pipeline = |key: Option<&MeshPipelineKey>, depth: bool, reuse_depth: bool| {
                key.filter(|_| active).and_then(|key| {
                    pipelines
                        .specialize(
                            &cache,
                            specializer,
                            key_for(*key, depth, reuse_depth),
                            layout,
                        )
                        .ok()
                })
            };
            let ready = |id: CachedRenderPipelineId| cache.get_render_pipeline(id).is_some();
            if light.is_some() {
                if p != FAR_TERRAIN
                    && let Some(phase) = shadows.get_mut(&retained)
                {
                    let keys = pipeline(light_keys.get(&retained), true, false).map(|pipeline| {
                        let batch = ShadowBatchSetKey {
                            pipeline,
                            draw_function: shadow_draws[p],
                            material_bind_group_index: None,
                            slabs: MeshSlabs::default(),
                        };
                        (batch, ShadowBinKey { asset_id })
                    });
                    keep(phase, keys, item(entity));
                }
                continue;
            }
            let prepass = if p == 0 && depth.contains_key(&retained) {
                pipeline(depth_keys.get(&retained), true, false)
            } else {
                None
            };
            let mut main = pipeline(keys.get(&retained), false, false);
            // Terrain shading keeps the prepass depth without rewriting it. Until the
            // prepass and this variant are both compiled, the writing pipeline stays.
            if prepass.is_some_and(ready) && main.is_some() {
                main = pipeline(keys.get(&retained), false, true)
                    .filter(|&id| ready(id))
                    .or(main);
            }
            if p == 1 || p == 3 {
                if let Some(phase) = alpha.get_mut(&retained) {
                    let keys = main.map(|pipeline| {
                        let batch = OpaqueNoLightmap3dBatchSetKey {
                            pipeline,
                            draw_function: main_draws[p],
                            material_bind_group_index: None,
                            slabs: MeshSlabs::default(),
                        };
                        (batch, OpaqueNoLightmap3dBinKey { asset_id })
                    });
                    keep(phase, keys, item(entity));
                }
            } else if let Some(phase) = opaque.get_mut(&retained) {
                let keys = main.map(|pipeline| {
                    let batch = Opaque3dBatchSetKey {
                        pipeline,
                        draw_function: main_draws[p],
                        material_bind_group_index: None,
                        slabs: MeshSlabs::default(),
                        lightmap_slab: None,
                    };
                    (batch, Opaque3dBinKey { asset_id })
                });
                keep(phase, keys, item(entity));
            }
            if p == 0
                && let Some(phase) = depth.get_mut(&retained)
            {
                let keys = prepass.map(|pipeline| {
                    let batch = OpaqueNoLightmap3dBatchSetKey {
                        pipeline,
                        draw_function: depth_draw,
                        material_bind_group_index: None,
                        slabs: MeshSlabs::default(),
                    };
                    (batch, OpaqueNoLightmap3dBinKey { asset_id })
                });
                keep(phase, keys, item(entity));
            }
        }
    }
}
/// Keeps one retained non-mesh item in `phase` under `keys`, or removes it.
fn keep<BPI: BinnedPhaseItem>(
    phase: &mut BinnedRenderPhase<BPI>,
    keys: Option<(BPI::BatchSetKey, BPI::BinKey)>,
    (entity, main): (Entity, MainEntity),
) {
    let Some(keys) = keys else {
        phase.remove(main);
        return;
    };
    if phase
        .non_mesh_items
        .get(&keys)
        .is_some_and(|items| items.entities.get(&main) == Some(&entity))
    {
        return;
    }
    // `add` alone would leave the item in its previous bin as well.
    phase.remove(main);
    phase.add(
        keys.0,
        keys.1,
        (entity, main),
        InputUniformIndex::default(),
        BinnedRenderPhaseType::NonMesh,
    );
}
type MainDraw<const P: usize> = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetSections,
    SetSectionMaterial<P>,
    DrawSections<P, false>,
);
type DepthDraw<const P: usize> = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    SetSections,
    SetSectionMaterial<P>,
    DrawSections<P, true>,
);
struct SetSections;
impl<I: PhaseItem> RenderCommand<I> for SetSections {
    type Param = SRes<SectionGpu>;
    type ViewQuery = ();
    type ItemQuery = ();
    fn render<'w>(
        _: &I,
        _: (),
        _: Option<()>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        pass.set_bind_group(2, &gpu.into_inner().bind_group, &[]);
        RenderCommandResult::Success
    }
}
struct SetSectionMaterial<const P: usize>;
impl<I: PhaseItem, const P: usize> RenderCommand<I> for SetSectionMaterial<P> {
    type Param = (
        SRes<SectionMaterials>,
        SRes<ErasedRenderAssets<PreparedMaterial>>,
        SRes<MaterialBindGroupAllocators>,
    );
    type ViewQuery = ();
    type ItemQuery = ();
    fn render<'w>(
        _: &I,
        _: (),
        _: Option<()>,
        (ids, materials, allocators): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(ids) = &ids.into_inner().0 else {
            return RenderCommandResult::Skip;
        };
        let Some(material) = materials.into_inner().get(ids[P]) else {
            return RenderCommandResult::Skip;
        };
        let Some(allocator) = allocators.into_inner().get(&ids[P].type_id()) else {
            return RenderCommandResult::Skip;
        };
        let Some(group) = allocator
            .get(material.binding.group)
            .and_then(|g| g.bind_group())
        else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(3, group, &[]);
        RenderCommandResult::Success
    }
}
struct DrawSections<const P: usize, const DEPTH: bool>;
impl<I: PhaseItem, const P: usize, const DEPTH: bool> RenderCommand<I> for DrawSections<P, DEPTH> {
    type Param = SRes<SectionGpu>;
    type ViewQuery = Entity;
    type ItemQuery = ();
    fn render<'w>(
        _: &I,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<()>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let gpu = gpu.into_inner();
        let Some(draws) = gpu.ranges.get(&view) else {
            return RenderCommandResult::Skip;
        };
        let range = draws.ranges[P].clone();
        // Late depth passes repeat the phase: its sections are already in the target.
        if range.is_empty()
            || (DEPTH && draws.depth_drawn.fetch_or(1 << P, Ordering::Relaxed) & (1 << P) != 0)
        {
            return RenderCommandResult::Skip;
        }
        let arena = &gpu.arenas[range_pass(P)];
        pass.set_vertex_buffer(0, arena.vertices.buffer.slice(..));
        pass.set_index_buffer(
            arena.indices.buffer.slice(..),
            if INDEX_BYTES[range_pass(P)] == 2 {
                IndexFormat::Uint16
            } else {
                IndexFormat::Uint32
            },
        );
        if gpu.indirect_first_instance {
            pass.multi_draw_indexed_indirect(
                &gpu.indirect,
                (range.start * 20) as u64,
                u32::try_from(range.len()).expect("section draw count"),
            );
        } else {
            for a in &gpu.args[range] {
                pass.draw_indexed(a[2]..a[2] + a[0], a[3] as i32, a[4]..a[4] + 1);
            }
        }
        RenderCommandResult::Success
    }
}
