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
use std::{ops::Range, sync::Arc};

pub(crate) struct PassGeometry {
    pub vertices: Vec<u8>,
    pub indices: Vec<u8>,
    count: u32,
    min: Vec3,
    max: Vec3,
}
impl PassGeometry {
    fn new(vertices: Vec<u8>, mut indices: Vec<u8>, count: usize, min: Vec3, max: Vec3) -> Self {
        indices.resize(indices.len().next_multiple_of(4), 0);
        Self {
            vertices,
            indices,
            count: u32::try_from(count).expect("section index count"),
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
        Self::new(
            bytemuck::cast_slice(&geometry.vertices).to_vec(),
            bytemuck::cast_slice(&geometry.indices).to_vec(),
            geometry.indices.len(),
            min,
            max,
        )
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
struct SectionIds([MainEntity; 5]);

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
                (
                    prepare_sections,
                    queue_sections::<0>,
                    queue_sections::<1>,
                    queue_sections::<2>,
                    queue_sections::<3>,
                    queue_sections::<4>,
                )
                    .chain()
                    .in_set(RenderSystems::QueueMeshes),
            );
        register_draws::<0>(render);
        register_draws::<1>(render);
        register_draws::<2>(render);
        register_draws::<3>(render);
        register_draws::<4>(render);
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
        ]
    });
    commands.insert_resource(SectionMaterials(materials));
}
#[derive(Resource)]
struct SectionMaterials(Option<[UntypedAssetId; 5]>);

/// Sparse columns map to dense slots; no hashing in the per-view section loop.
struct Column {
    slots: [Option<usize>; 32],
    mask: u32,
}
struct Slot {
    visible: bool,
    origin: Vec3,
    min: Vec3,
    max: Vec3,
    passes: [Option<Allocation>; 5],
}
struct Allocation {
    vertices: Range<u64>,
    indices: Range<u64>,
    count: u32,
}
struct ArenaBuffer {
    buffer: Buffer,
    capacity: u64,
    end: u64,
    free: Vec<Range<u64>>,
    usage: BufferUsages,
}
impl ArenaBuffer {
    fn new(device: &RenderDevice, usage: BufferUsages) -> Self {
        let capacity = 4096;
        Self {
            buffer: device.create_buffer(&BufferDescriptor {
                label: Some("section arena"),
                size: capacity,
                usage: usage | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            capacity,
            end: 0,
            free: Vec::new(),
            usage,
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
        if self.end <= self.capacity {
            return;
        }
        let capacity = self.end.next_power_of_two();
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("section arena"),
            size: capacity,
            usage: self.usage | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &buffer, 0, self.capacity);
        self.buffer = buffer;
        self.capacity = capacity;
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
    free: Vec<usize>,
    origins: Buffer,
    origins_capacity: usize,
    bind_group: BindGroup,
    layout: BindGroupLayoutDescriptor,
    layouts: [MeshVertexBufferLayoutRef; 2],
    indirect: Buffer,
    indirect_capacity: usize,
    args: Vec<[u32; 5]>,
    ranges: HashMap<Entity, [Range<usize>; 5]>,
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
        let origins = device.create_buffer(&BufferDescriptor {
            label: Some("section origins"),
            size: 16,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(
            "section origins",
            &device.create_bind_group_layout(layout.label.as_ref(), &layout.entries),
            &[BindGroupEntry {
                binding: 32,
                resource: origins.as_entire_binding(),
            }],
        );
        Self {
            arenas: std::array::from_fn(|_| Arena {
                vertices: ArenaBuffer::new(device, BufferUsages::VERTEX),
                indices: ArenaBuffer::new(device, BufferUsages::INDEX),
            }),
            columns: default(),
            slots: Vec::new(),
            free: Vec::new(),
            origins,
            origins_capacity: 1,
            bind_group,
            layout,
            layouts,
            indirect: device.create_buffer(&BufferDescriptor {
                label: Some("section draws"),
                size: 20,
                usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            indirect_capacity: 1,
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
        }
    }
    fn remove(&mut self, column: IVec2) {
        if let Some(column) = self.columns.remove(&column) {
            for slot in column.slots.into_iter().flatten() {
                for pass in 0..5 {
                    self.release(slot, pass);
                }
                self.slots[slot] = None;
                self.free.push(slot);
            }
        }
    }
}
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
    mut draws: Local<[Vec<[u32; 5]>; 5]>,
) {
    let gpu = &mut *gpu;
    let mut uploads = Vec::new();
    for op in feed.ops.drain(..) {
        match op {
            SectionOp::Clear => {
                gpu.columns.clear();
                gpu.slots.clear();
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
                let c = gpu.columns.entry(column).or_insert(Column {
                    slots: [None; 32],
                    mask,
                });
                c.mask = mask;
                for (section, slot) in c.slots.iter().enumerate() {
                    if let Some(s) = slot.and_then(|i| gpu.slots[i].as_mut()) {
                        s.visible = mask & (1 << section) != 0;
                    }
                }
            }
            SectionOp::Replace {
                column,
                section,
                origin,
                geometry,
            } => {
                if geometry.iter().all(Option::is_none) {
                    if let Some(slot) = gpu
                        .columns
                        .get_mut(&column)
                        .and_then(|c| c.slots[section].take())
                    {
                        for pass in 0..5 {
                            gpu.release(slot, pass);
                        }
                        gpu.slots[slot] = None;
                        gpu.free.push(slot);
                    }
                    continue;
                }
                let c = gpu.columns.entry(column).or_insert(Column {
                    slots: [None; 32],
                    mask: u32::MAX,
                });
                let visible = c.mask & (1 << section) != 0;
                let slot = *c.slots[section].get_or_insert_with(|| {
                    gpu.free.pop().unwrap_or_else(|| {
                        gpu.slots.push(None);
                        gpu.slots.len() - 1
                    })
                });
                for pass in 0..5 {
                    gpu.release(slot, pass);
                }
                let mut record = Slot {
                    visible,
                    origin,
                    min: Vec3::MAX,
                    max: Vec3::MIN,
                    passes: std::array::from_fn(|_| None),
                };
                for (pass, geometry) in geometry.into_iter().enumerate() {
                    if let Some(g) = geometry {
                        record.min = record.min.min(g.min + origin);
                        record.max = record.max.max(g.max + origin);
                        let a = Allocation {
                            vertices: gpu.arenas[pass].vertices.allocate(g.vertices.len() as u64),
                            indices: gpu.arenas[pass].indices.allocate(g.indices.len() as u64),
                            count: g.count,
                        };
                        uploads.push((slot, pass, a.vertices.start, a.indices.start, g));
                        record.passes[pass] = Some(a);
                    }
                }
                gpu.slots[slot] = Some(record);
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
        if gpu
            .arenas
            .iter()
            .any(|a| a.vertices.end > a.vertices.capacity || a.indices.end > a.indices.capacity)
        {
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
        if gpu.slots.len() > gpu.origins_capacity {
            gpu.origins_capacity = gpu.slots.len().next_power_of_two();
            gpu.origins = device.create_buffer(&BufferDescriptor {
                label: Some("section origins"),
                size: (gpu.origins_capacity * 16) as u64,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            gpu.bind_group = device.create_bind_group(
                "section origins",
                &device.create_bind_group_layout(gpu.layout.label.as_ref(), &gpu.layout.entries),
                &[BindGroupEntry {
                    binding: 32,
                    resource: gpu.origins.as_entire_binding(),
                }],
            );
        }
        let origins: Vec<[f32; 4]> = gpu
            .slots
            .iter()
            .map(|s| {
                s.as_ref()
                    .map_or([0.0; 4], |s| s.origin.extend(0.0).to_array())
            })
            .collect();
        if !origins.is_empty() {
            queue.write_buffer(&gpu.origins, 0, bytemuck::cast_slice(&origins));
        }
    }
    gpu.args.clear();
    gpu.ranges.clear();
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
        let clip = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let rows = clip.transpose().to_cols_array_2d().map(Vec4::from_array);
        let planes = [
            rows[3] + rows[0],
            rows[3] - rows[0],
            rows[3] + rows[1],
            rows[3] - rows[1],
            rows[3] - rows[2],
        ];
        let enabled: [bool; 5] = std::array::from_fn(|p| {
            shadow_layers.as_ref().map_or(p != 4, |layers| {
                if p == 0 {
                    false
                } else if p == 4 {
                    layers.intersects(&RenderLayers::layer(crate::SHADOW_LAYER))
                } else {
                    layers.intersects(&RenderLayers::from_layers(&[
                        0,
                        crate::visibility::OCCLUDED_LAYER,
                    ]))
                }
            })
        });
        for buffer in draws.iter_mut() {
            buffer.clear();
        }
        for (slot, s) in gpu
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i, s)))
        {
            if shadow_layers.is_none() && !feed.conservative && !s.visible {
                continue;
            }
            let center = (s.min + s.max) * 0.5;
            let extent = (s.max - s.min) * 0.5;
            if planes[..if shadow_layers.is_some() { 4 } else { 5 }]
                .iter()
                .any(|p| p.truncate().dot(center) + p.w + p.truncate().abs().dot(extent) < 0.0)
            {
                continue;
            }
            for p in 0..5 {
                if !enabled[p] {
                    continue;
                }
                let Some(a) = &s.passes[p] else {
                    continue;
                };
                draws[p].push([
                    a.count,
                    1,
                    u32::try_from(a.indices.start / if p == 0 || p == 4 { 2 } else { 4 })
                        .expect("arena index offset"),
                    i32::try_from(a.vertices.start / if p == 2 || p == 3 { 16 } else { 8 })
                        .expect("arena vertex offset") as u32,
                    u32::try_from(slot).expect("section slot"),
                ]);
            }
        }
        let ranges = std::array::from_fn(|p| {
            let start = gpu.args.len();
            gpu.args.extend_from_slice(&draws[p]);
            start..gpu.args.len()
        });
        gpu.ranges.insert(entity, ranges);
    }
    if gpu.args.len() > gpu.indirect_capacity {
        gpu.indirect_capacity = gpu.args.len().next_power_of_two();
        gpu.indirect = device.create_buffer(&BufferDescriptor {
            label: Some("section draws"),
            size: (gpu.indirect_capacity * 20) as u64,
            usage: BufferUsages::INDIRECT | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }
    if !gpu.args.is_empty() && gpu.indirect_first_instance {
        queue.write_buffer(&gpu.indirect, 0, bytemuck::cast_slice(&gpu.args));
    }
}

struct SectionSpecializer {
    mesh: MaterialPipeline,
    prepass: PrepassPipeline,
    properties: Arc<MaterialProperties>,
    layout: BindGroupLayoutDescriptor,
}
impl SpecializedMeshPipeline for SectionSpecializer {
    type Key = (bool, ErasedMaterialPipelineKey);
    fn specialize(
        &self,
        (depth, key): Self::Key,
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
#[allow(clippy::too_many_arguments)]
fn queue_sections<const P: usize>(
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
    let Some(material) = prepared.get(materials[P]) else {
        return;
    };
    let specializer = SectionSpecializer {
        mesh: MaterialPipeline {
            mesh_pipeline: mesh.clone(),
        },
        prepass: prepass.clone(),
        properties: material.properties.clone(),
        layout: gpu.layout.clone(),
    };
    let layout = &gpu.layouts[usize::from(P == 2 || P == 3)];
    let main = ids.0[P];
    for (entity, view, light) in &views {
        let retained = view.retained_view_entity;
        if let Some(phase) = opaque.get_mut(&retained) {
            phase.remove(main);
        }
        if let Some(phase) = alpha.get_mut(&retained) {
            phase.remove(main);
        }
        if let Some(phase) = depth.get_mut(&retained) {
            phase.remove(main);
        }
        if let Some(phase) = shadows.get_mut(&retained) {
            phase.remove(main);
        }
        if gpu.ranges.get(&entity).is_none_or(|r| r[P].is_empty()) {
            continue;
        }
        let topology = MeshPipelineKey::from_primitive_topology_and_strip_index(
            PrimitiveTopology::TriangleList,
            None,
        );
        let key_for = |view_key: MeshPipelineKey, depth: bool| {
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
                ErasedMaterialPipelineKey {
                    mesh_key: ErasedMeshPipelineKey::new(key),
                    material_key: material.properties.material_key.clone(),
                    type_id: materials[P].type_id(),
                },
            )
        };
        let asset_id = AssetId::<Mesh>::default().untyped();
        if light.is_some() {
            let (Some(phase), Some(key)) = (shadows.get_mut(&retained), light_keys.get(&retained))
            else {
                continue;
            };
            if let Ok(pipeline) =
                pipelines.specialize(&cache, &specializer, key_for(*key, true), layout)
            {
                phase.add(
                    ShadowBatchSetKey {
                        pipeline,
                        draw_function: sd.read().id::<DepthDraw<P>>(),
                        material_bind_group_index: None,
                        slabs: MeshSlabs::default(),
                    },
                    ShadowBinKey { asset_id },
                    (entity, main),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                );
            }
            continue;
        }
        if let Some(key) = keys.get(&retained) {
            if let Ok(pipeline) =
                pipelines.specialize(&cache, &specializer, key_for(*key, false), layout)
            {
                let batch = Opaque3dBatchSetKey {
                    pipeline,
                    draw_function: if P == 1 || P == 3 {
                        ad.read().id::<MainDraw<P>>()
                    } else {
                        od.read().id::<MainDraw<P>>()
                    },
                    material_bind_group_index: None,
                    slabs: MeshSlabs::default(),
                    lightmap_slab: None,
                };
                if P == 1 || P == 3 {
                    if let Some(phase) = alpha.get_mut(&retained) {
                        phase.add(
                            OpaqueNoLightmap3dBatchSetKey {
                                pipeline: batch.pipeline,
                                draw_function: batch.draw_function,
                                material_bind_group_index: None,
                                slabs: MeshSlabs::default(),
                            },
                            OpaqueNoLightmap3dBinKey { asset_id },
                            (entity, main),
                            InputUniformIndex::default(),
                            BinnedRenderPhaseType::NonMesh,
                        );
                    }
                } else if let Some(phase) = opaque.get_mut(&retained) {
                    phase.add(
                        batch,
                        Opaque3dBinKey { asset_id },
                        (entity, main),
                        InputUniformIndex::default(),
                        BinnedRenderPhaseType::NonMesh,
                    );
                }
            }
        }
        if P == 0 {
            if let (Some(phase), Some(key)) = (depth.get_mut(&retained), depth_keys.get(&retained))
            {
                if let Ok(pipeline) =
                    pipelines.specialize(&cache, &specializer, key_for(*key, true), layout)
                {
                    phase.add(
                        OpaqueNoLightmap3dBatchSetKey {
                            pipeline,
                            draw_function: dd.read().id::<DepthDraw<P>>(),
                            material_bind_group_index: None,
                            slabs: MeshSlabs::default(),
                        },
                        OpaqueNoLightmap3dBinKey { asset_id },
                        (entity, main),
                        InputUniformIndex::default(),
                        BinnedRenderPhaseType::NonMesh,
                    );
                }
            }
        }
    }
}
type MainDraw<const P: usize> = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetSections,
    SetSectionMaterial<P>,
    DrawSections<P>,
);
type DepthDraw<const P: usize> = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    SetSections,
    SetSectionMaterial<P>,
    DrawSections<P>,
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
struct DrawSections<const P: usize>;
impl<I: PhaseItem, const P: usize> RenderCommand<I> for DrawSections<P> {
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
        let Some(ranges) = gpu.ranges.get(&view) else {
            return RenderCommandResult::Skip;
        };
        let range = ranges[P].clone();
        if range.is_empty() {
            return RenderCommandResult::Skip;
        }
        pass.set_vertex_buffer(0, gpu.arenas[P].vertices.buffer.slice(..));
        pass.set_index_buffer(
            gpu.arenas[P].indices.buffer.slice(..),
            if P == 0 || P == 4 {
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
