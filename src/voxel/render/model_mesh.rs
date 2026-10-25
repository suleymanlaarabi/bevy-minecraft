//! Explicit model vertices are confined to non-greedy shapes and translucency.
use super::{
    meshing::{Padded, TerrainGeometry},
    models::{self, ModelQuad},
};
use crate::voxel::{
    VoxelKind,
    coordinates::{CHUNK_SIZE, SECTION_HEIGHT},
};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, MeshVertexAttribute, VertexFormat},
    prelude::*,
    render::render_resource::PrimitiveTopology,
    shape::Aabb3d,
};
use std::sync::Arc;

/// Packed model vertex, decoded by `model_vertex.wesl` (16 bytes instead of 48):
/// - words 0-1: position as 15-bit fixed point per axis over `[-2, 30)` (1/1024 block),
///   octahedral normal (2 x 8 bits) and tint palette index (3 bits);
/// - word 2: UV as two 16-bit unorms;
/// - word 3: model face-table slot (13 bits), corner light (12 bits), and water
///   flag plus column depth (5 bits).
pub(crate) const ATTRIBUTE_MODEL: MeshVertexAttribute =
    MeshVertexAttribute::new("BlockModel", 0x626c6f636b5f6d32, VertexFormat::Uint32x4);

const POSITION_BIAS: f32 = 2.0;
const POSITION_SCALE: f32 = 1024.0;
const POSITION_MAX: u32 = (1 << 15) - 1;

/// Unpacked geometry while meshing; [`ModelGeometry::into_mesh`] packs it.
#[derive(Default)]
pub(crate) struct ModelGeometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    /// Face-table slot, corner light, tint palette index and water flags.
    pub metadata: Vec<[u32; 4]>,
    pub indices: Vec<u32>,
    /// Owning cell centers for translucent quads only, before any visual offsets.
    sort_cells: Vec<Vec3>,
}

/// Octahedral encoding mapped to 0..=254, so axis-aligned normals stay exact.
fn octahedral(normal: Vec3) -> [u32; 2] {
    bevy::mesh::octahedral_encode_signed(normal.normalize_or(Vec3::Y))
        .to_array()
        .map(|value| ((value.clamp(-1.0, 1.0) * 0.5 + 0.5) * 254.0).round() as u32)
}

fn pack_vertex(position: [f32; 3], normal: [f32; 3], uv: [f32; 2], meta: [u32; 4]) -> [u32; 4] {
    let [x, y, z] = position.map(|value| {
        (((value + POSITION_BIAS) * POSITION_SCALE).round() as i64).clamp(0, POSITION_MAX as i64)
            as u32
    });
    let [nx, ny] = octahedral(Vec3::from_array(normal));
    let [u, v] = uv.map(|value| (value.clamp(0.0, 1.0) * 65535.0).round() as u32);
    let [slot, light, tint, flags] = meta;
    debug_assert!(slot < 1 << 13, "model face slot must fit in 13 bits");
    [
        x | y << 15 | (z & 3) << 30,
        z >> 2 | nx << 13 | ny << 21 | (tint & 7) << 29,
        u | v << 16,
        (slot & 0x1fff) | (light & 0xfff) << 13 | (flags & 0x1f) << 25,
    ]
}
impl ModelGeometry {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
    #[cfg(feature = "dev")]
    pub fn counts(&self) -> (usize, usize) {
        (self.positions.len(), self.indices.len() / 3)
    }
    pub fn quad(
        &mut self,
        q: &ModelQuad,
        offset: Vec3,
        light: [u16; 4],
        tint: u8,
        slot: Option<u32>,
    ) {
        let first = self.positions.len() as u32;
        let tint = u32::from(tint);
        self.positions.extend(
            q.positions
                .map(|p| (Vec3::from_array(p) + offset).to_array()),
        );
        self.normals.extend(
            [if q.shade_up {
                [0.0, 1.0, 0.0]
            } else {
                q.normal
            }; 4],
        );
        self.uv.extend(q.uv);
        self.metadata
            .extend(light.map(|l| [slot.unwrap_or(q.texture), u32::from(l), tint, 0]));
        self.indices
            .extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    /// A translucent quad and the cell it sorts with, kept in step by construction.
    fn translucent_quad(
        &mut self,
        q: &ModelQuad,
        offset: Vec3,
        light: [u16; 4],
        tint: u8,
        cell: Vec3,
    ) {
        self.quad(q, offset, light, tint, None);
        self.sort_cells.push(cell);
    }
    pub fn append_water(&mut self, water: TerrainGeometry) {
        for (quad, indices) in water
            .vertices
            .chunks_exact(4)
            .zip(water.indices.chunks_exact(6))
        {
            let first = self.positions.len() as u32;
            let (axis, positive) = super::meshing::face_direction(quad[0]);
            let mut normal = Vec3::ZERO;
            normal[axis] = if positive { 1.0 } else { -1.0 };
            // Recover the owning water cells (their center for a merged face)
            // before lowering the free surface, so it sorts with its blocks.
            let center = quad
                .iter()
                .map(|&v| TerrainGeometry::position(v))
                .sum::<Vec3>()
                * 0.25;
            self.sort_cells.push(center - normal * 0.5);
            for &v in quad {
                let mut position = TerrainGeometry::position(v);
                if v[1] & (1 << 16) != 0 {
                    position.y -= 1.0 / 8.0;
                }
                self.positions.push(position.to_array());
                self.normals.push(normal.to_array());
                self.uv.push([0.0; 2]);
                self.metadata
                    .push([0, v[1] & 4095, 0, 1 | (((v[1] >> 12) & 15) << 1)]);
            }
            let base = u32::from(indices[0]);
            self.indices
                .extend(indices.iter().map(|&i| first + u32::from(i) - base));
        }
    }
    pub fn sort_data(&self) -> SortData {
        // Translucent geometry only grows through `translucent_quad` and `append_water`.
        assert_eq!(self.sort_cells.len() * 4, self.positions.len());
        let quads = self
            .positions
            .chunks_exact(4)
            .zip(self.indices.chunks_exact(6))
            .zip(&self.sort_cells)
            .map(|((p, indices), &cell)| {
                let center = p.iter().map(|&v| Vec3::from_array(v)).sum::<Vec3>() * 0.25;
                (cell, center, <[u32; 6]>::try_from(indices).unwrap())
            })
            .collect::<Vec<_>>();
        SortData(Arc::from(quads))
    }
    pub fn into_dynamic_mesh(self) -> Mesh {
        let mut mesh = self.into_mesh();
        mesh.asset_usage = RenderAssetUsages::default();
        mesh
    }
    pub fn into_mesh(self) -> Mesh {
        let (min, max) = self
            .positions
            .iter()
            .map(|&p| Vec3::from_array(p))
            .fold((Vec3::MAX, Vec3::MIN), |(min, max), p| {
                (min.min(p), max.max(p))
            });
        let vertices = self
            .positions
            .iter()
            .zip(&self.normals)
            .zip(&self.uv)
            .zip(&self.metadata)
            .map(|(((&position, &normal), &uv), &meta)| pack_vertex(position, normal, uv, meta))
            .collect::<Vec<_>>();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(ATTRIBUTE_MODEL, vertices)
        .with_inserted_indices(compact_indices(self.indices, self.positions.len()));
        // Bevy derives bounds from `ATTRIBUTE_POSITION`, which packed meshes lack.
        mesh.final_aabb = min.cmple(max).all().then(|| Aabb3d::from_min_max(min, max));
        mesh
    }
}
/// 16-bit indices whenever the vertices allow it: half the index bandwidth.
pub(crate) fn compact_indices(indices: Vec<u32>, vertices: usize) -> Indices {
    if vertices <= usize::from(u16::MAX) + 1 {
        Indices::U16(indices.into_iter().map(|index| index as u16).collect())
    } else {
        Indices::U32(indices)
    }
}

/// Owning cells, face centers and indices of a translucent section.
#[derive(Clone, Component)]
pub(crate) struct SortData(pub Arc<[(Vec3, Vec3, [u32; 6])]>);
impl SortData {
    /// Order blocks first, then their faces: a water face must never jump in
    /// front of ice in a nearer block merely because their face centers differ.
    pub fn indices(&self, eye: Vec3) -> Vec<u32> {
        self.sorted(eye, |index| index)
    }
    /// Sorted indices at the narrowest width the vertex count allows.
    pub fn mesh_indices(&self, eye: Vec3) -> Indices {
        if self.vertex_count() <= usize::from(u16::MAX) + 1 {
            Indices::U16(self.sorted(eye, |index| index as u16))
        } else {
            Indices::U32(self.indices(eye))
        }
    }
    fn sorted<T>(&self, eye: Vec3, narrow: impl Fn(u32) -> T) -> Vec<T> {
        // Equal-sized grid cells have a consistent radial back-to-front order
        // along each ray. Cache integer keys for the non-negative distances.
        let mut order = self
            .0
            .iter()
            .enumerate()
            .map(|(index, (cell, center, _))| {
                let cell = u64::from(cell.distance_squared(eye).to_bits());
                let center = u64::from(center.distance_squared(eye).to_bits());
                (cell << 32 | center, index as u32)
            })
            .collect::<Vec<_>>();
        order.sort_unstable_by(|a, b| b.cmp(a));
        let mut indices = Vec::with_capacity(self.0.len() * 6);
        indices.extend(
            order
                .into_iter()
                .flat_map(|(_, quad)| self.0[quad as usize].2.map(&narrow)),
        );
        indices
    }
    pub fn vertex_count(&self) -> usize {
        self.0.len() * 4
    }
}

/// Model quads of one section, split by how the GPU must blend them.
#[derive(Default)]
pub(super) struct ModelSection {
    /// Fully opaque textures: no discard, so early depth tests stay enabled.
    pub(super) opaque: Option<ModelGeometry>,
    /// Alpha-tested textures (plants, torches, rails...).
    pub(super) cutout: Option<ModelGeometry>,
    pub(super) translucent: Option<ModelGeometry>,
}

pub(super) fn section(padded: &Padded, origin: IVec3) -> ModelSection {
    let (mut opaque, mut cutout, mut translucent) = (
        ModelGeometry::default(),
        ModelGeometry::default(),
        ModelGeometry::default(),
    );
    let fallback = models::texture_names().is_empty();
    for y in 0..SECTION_HEIGHT {
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let local = IVec3::new(x, y, z);
                let sample = padded.get(local);
                if !sample.model() {
                    continue;
                }
                let world = origin + local;
                // Models and native shapes share the same block-local translation.
                let model_offset = sample.state.visual_offset(world);
                let light =
                    (u16::from(sample.light >> 4) * 4) | (u16::from(sample.light & 15) * 4 << 6);
                // Quads of one cell share few directions: light each exposed face once.
                let mut face_lights: [Option<[u16; 4]>; 6] = [None; 6];
                models::visit_quads(sample.state, world, |q| {
                    let cull = IVec3::from_array(q.cull);
                    if cull != IVec3::ZERO {
                        let adjacent = padded.get(local + cull);
                        if adjacent.occludes()
                            || (q.translucent && adjacent.kind() == sample.kind())
                        {
                            return;
                        }
                    }
                    let tint = if q.tinted {
                        models::tint_index(sample.kind())
                    } else {
                        0
                    };
                    let translation =
                        local.as_vec3() + if fallback { Vec3::ZERO } else { model_offset };
                    let n = Vec3::from_array(q.normal);
                    let axis = if n.y.abs() >= n.x.abs() && n.y.abs() >= n.z.abs() {
                        1
                    } else if n.x.abs() >= n.z.abs() {
                        0
                    } else {
                        2
                    };
                    let positive = n[axis] >= 0.0;
                    let face =
                        *face_lights[axis * 2 + usize::from(positive)].get_or_insert_with(|| {
                            let mut outward = IVec3::ZERO;
                            outward[axis] = if positive { 1 } else { -1 };
                            super::meshing::face_light(padded, (local + outward).to_array(), axis)
                        });
                    // Opaque states store zero light inside themselves: sample the exposed
                    // face, exactly as greedy terrain does, rather than darkening the model.
                    let face = q.positions.map(|p| {
                        let u = p[(axis + 1) % 3].clamp(0.0, 1.0);
                        let v = p[(axis + 2) % 3].clamp(0.0, 1.0);
                        let weights = [(1.0 - u) * (1.0 - v), u * (1.0 - v), u * v, (1.0 - u) * v];
                        let channel = |shift: u16| {
                            let value = face
                                .iter()
                                .zip(weights)
                                .map(|(&value, weight)| ((value >> shift) & 63) as f32 * weight)
                                .sum::<f32>()
                                .round() as u16;
                            value.max((light >> shift) & 63)
                        };
                        channel(0) | (channel(6) << 6)
                    });
                    if q.translucent {
                        let cell = local.as_vec3() + Vec3::splat(0.5);
                        translucent.translucent_quad(q, translation, face, tint, cell);
                    } else if models::opaque_texture(q.texture) {
                        opaque.quad(q, translation, face, tint, None);
                    } else {
                        cutout.quad(q, translation, face, tint, None);
                    }
                });
            }
        }
    }
    let some = |geometry: ModelGeometry| (!geometry.is_empty()).then_some(geometry);
    ModelSection {
        opaque: some(opaque),
        cutout: some(cutout),
        translucent: some(translucent),
    }
}
pub(crate) fn item_geometry(kind: VoxelKind) -> ModelGeometry {
    state_geometry(kind.default_state(), IVec3::ZERO)
}
pub(crate) fn state_geometry(state: crate::voxel::VoxelState, world: IVec3) -> ModelGeometry {
    let kind = state.kind();
    let mut geometry = ModelGeometry::default();
    models::visit_quads(state, world, |q| {
        geometry.quad(
            q,
            if models::texture_names().is_empty() {
                Vec3::ZERO
            } else {
                state.visual_offset(world)
            },
            [60; 4],
            if q.tinted {
                models::tint_index(kind)
            } else {
                0
            },
            None,
        )
    });
    geometry
}
