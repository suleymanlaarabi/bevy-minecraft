//! Explicit model vertices are confined to non-greedy shapes and translucency.
use super::{
    meshing::{Padded, TerrainGeometry},
    models::{self, ModelQuad},
};
use crate::voxel::VoxelKind;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, MeshVertexAttribute, VertexFormat},
    prelude::*,
    render::render_resource::PrimitiveTopology,
    shape::Aabb3d,
};
use hollow_worldgen::coordinates::{CHUNK_SIZE, SECTION_HEIGHT};
use std::sync::Arc;

/// Packed model vertex, decoded by `model_vertex.wesl` (16 bytes instead of 48):
/// - words 0-1: position as 15-bit fixed point per axis over `[-2, 30)` (1/1024 block),
///   octahedral normal (2 x 8 bits) and tint palette index (3 bits);
/// - word 2: UV as two 16-bit fixed-point values in [-8, 8), at 1/4096 precision;
/// - word 3: model face-table slot (13 bits), corner light (12 bits), and water
///   flag plus column depth (5 bits).
pub(crate) const ATTRIBUTE_MODEL: MeshVertexAttribute =
    MeshVertexAttribute::new("BlockModel", 0x626c6f636b5f6d32, VertexFormat::Uint32x4);

const POSITION_BIAS: f32 = 2.0;
pub(super) const POSITION_SCALE: f32 = 1024.0;
const POSITION_MAX: u32 = (1 << 15) - 1;

/// GPU-ready geometry, with bounds and sort keys from the original positions.
pub(crate) struct ModelGeometry {
    vertices: Vec<[u32; 4]>,
    pub indices: Vec<u32>,
    min: Vec3,
    max: Vec3,
    sort_quads: Vec<SortQuad>,
}

impl Default for ModelGeometry {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            min: Vec3::MAX,
            max: Vec3::MIN,
            sort_quads: Vec::new(),
        }
    }
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
    let [u, v] = uv.map(|value| {
        debug_assert!(
            value.is_finite() && (-8.0..8.0).contains(&value),
            "model UV must fit the packed range"
        );
        ((value + 8.0) * 4096.0).round().clamp(0.0, 65535.0) as u32
    });
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
        (self.vertices.len(), self.indices.len() / 3)
    }
    fn vertex(&mut self, position: Vec3, normal: [f32; 3], uv: [f32; 2], meta: [u32; 4]) {
        self.min = self.min.min(position);
        self.max = self.max.max(position);
        self.vertices
            .push(pack_vertex(position.to_array(), normal, uv, meta));
    }

    pub fn quad(
        &mut self,
        q: &ModelQuad,
        offset: Vec3,
        light: [u16; 4],
        tint: u8,
        cell: Option<Vec3>,
    ) {
        let first = self.vertices.len() as u32;
        let positions = q.positions.map(|p| Vec3::from_array(p) + offset);
        let normal = if q.shade_up {
            [0.0, 1.0, 0.0]
        } else {
            q.normal
        };
        for ((position, uv), light) in positions.into_iter().zip(q.uv).zip(light) {
            self.vertex(
                position,
                normal,
                uv,
                [
                    q.texture,
                    u32::from(light),
                    u32::from(tint),
                    u32::from(q.effect) | if q.light_emission == 15 { 4 } else { 0 },
                ],
            );
        }
        self.indices.extend(quad_indices(first / 4, false));
        if let Some(cell) = cell {
            self.sort_quads.push(SortQuad {
                cell,
                center: positions.into_iter().sum::<Vec3>() * 0.25,
                reversed: false,
            });
        }
    }
    pub fn append_water(&mut self, water: TerrainGeometry) {
        for (quad, indices) in water
            .vertices
            .chunks_exact(4)
            .zip(water.indices.chunks_exact(6))
        {
            let first = self.vertices.len() as u32;
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
            let positions = std::array::from_fn::<_, 4, _>(|i| {
                let v = quad[i];
                let mut position = TerrainGeometry::position(v);
                if v[1] & (1 << 16) != 0 {
                    position.y -= 1.0 / 9.0;
                }
                self.vertex(
                    position,
                    normal.to_array(),
                    [0.0; 2],
                    [
                        models::fluid_texture(1, false),
                        v[1] & 4095,
                        0,
                        1 | (((v[1] >> 12) & 15) << 1),
                    ],
                );
                position
            });
            let reversed = indices[1] > indices[2];
            debug_assert_eq!(
                indices,
                &quad_indices(u32::from(indices[0]) / 4, reversed).map(|i| i as u16)
            );
            self.sort_quads.push(SortQuad {
                cell: center - normal * 0.5,
                center: positions.into_iter().sum::<Vec3>() * 0.25,
                reversed,
            });
            let base = u32::from(indices[0]);
            self.indices
                .extend(indices.iter().map(|&i| first + u32::from(i) - base));
        }
    }
    pub fn take_sort_data(&mut self) -> SortData {
        // Translucent geometry records its original centers before vertex quantization.
        assert_eq!(self.sort_quads.len() * 4, self.vertices.len());
        SortData(Arc::from(std::mem::take(&mut self.sort_quads)))
    }
    pub fn into_mesh(self) -> Mesh {
        let vertex_count = self.vertices.len();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(ATTRIBUTE_MODEL, self.vertices)
        .with_inserted_indices(compact_indices(self.indices, vertex_count));
        // Bevy derives bounds from `ATTRIBUTE_POSITION`, which packed meshes lack.
        mesh.final_aabb = self
            .min
            .cmple(self.max)
            .all()
            .then(|| Aabb3d::from_min_max(self.min, self.max));
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

/// Every translucent quad owns four consecutive vertices. Reconstruct its six
/// indices instead of retaining 24 bytes of redundant indices for every quad.
fn quad_indices(quad: u32, reversed: bool) -> [u32; 6] {
    let first = quad * 4;
    let offsets = if reversed {
        [0, 2, 1, 0, 3, 2]
    } else {
        [0, 1, 2, 0, 2, 3]
    };
    offsets.map(|offset| first + offset)
}

struct SortQuad {
    cell: Vec3,
    center: Vec3,
    reversed: bool,
}

/// Owning cells, face centers and winding of a translucent section.
#[derive(Clone, Component)]
pub(crate) struct SortData(Arc<[SortQuad]>);
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
        thread_local! {
            static ORDER: std::cell::RefCell<Vec<(u64, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        ORDER.with_borrow_mut(|order| {
            // Equal-sized grid cells have a consistent radial back-to-front order
            // along each ray. Cache integer keys for the non-negative distances.
            order.clear();
            order.extend(self.0.iter().enumerate().map(|(index, quad)| {
                let cell = u64::from(quad.cell.distance_squared(eye).to_bits());
                let center = u64::from(quad.center.distance_squared(eye).to_bits());
                (cell << 32 | center, index as u32)
            }));
            order.sort_unstable_by(|a, b| b.cmp(a));
            let mut indices = Vec::with_capacity(self.0.len() * 6);
            indices.extend(order.iter().copied().flat_map(|(_, quad)| {
                quad_indices(quad, self.0[quad as usize].reversed).map(&narrow)
            }));
            indices
        })
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

pub(super) fn section(
    padded: &Padded,
    origin: IVec3,
    neighborhood: &super::meshing::ChunkNeighborhood,
    base: i32,
    water_depths: &mut Vec<u8>,
) -> ModelSection {
    let (mut opaque, mut cutout, mut translucent) = (
        ModelGeometry::default(),
        ModelGeometry::default(),
        ModelGeometry::default(),
    );
    let fallback = models::texture_names().is_empty();
    for y in 0..SECTION_HEIGHT {
        water_depths.clear();
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let local = IVec3::new(x, y, z);
                let sample = padded.get(local);
                let uneven_water = super::fluids::explicit_water(padded, local);
                if !sample.model() && !uneven_water {
                    continue;
                }
                let world = origin + local;
                // Models and native shapes share the same block-local translation.
                let model_offset = sample.state.visual_offset(world);
                let light =
                    (u16::from(sample.light >> 4) * 4) | (u16::from(sample.light & 15) * 4 << 6);
                // Quads of one cell share few directions: light each exposed face once.
                let mut face_lights: [Option<[u16; 4]>; 6] = [None; 6];
                let mut emit = |q: &ModelQuad| {
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
                        channel(0) | (channel(6).max(u16::from(q.light_emission) * 4) << 6)
                    });
                    if q.translucent {
                        let cell = local.as_vec3() + Vec3::splat(0.5);
                        translucent.quad(q, translation, face, tint, Some(cell));
                        if q.effect & 1 != 0 {
                            let depths = if n.y > 0.0 {
                                // Use the same column samples, rounding and corner order
                                // as greedy water, including across section boundaries.
                                let stride = CHUNK_SIZE + 2;
                                if water_depths.is_empty() {
                                    water_depths.extend((-1..=CHUNK_SIZE).flat_map(|z| {
                                        (-1..=CHUNK_SIZE).map(move |x| {
                                            super::meshing::water_depth(
                                                neighborhood,
                                                IVec3::new(x, base + y, z),
                                            )
                                        })
                                    }));
                                }
                                let corners =
                                    super::meshing::surface_depth(water_depths, stride, x, z);
                                q.positions.map(|p| {
                                    // Clipped fluid faces introduce vertices between
                                    // cell corners; interpolate on the original two
                                    // triangles, whose shared diagonal runs from 0 to 2.
                                    let (x, z) = (p[0].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0));
                                    let weights = if z >= x {
                                        [1.0 - z, z - x, x, 0.0]
                                    } else {
                                        [1.0 - x, 0.0, z, x - z]
                                    };
                                    corners
                                        .iter()
                                        .zip(weights)
                                        .map(|(&depth, weight)| f32::from(depth) * weight)
                                        .sum::<f32>()
                                        .round() as u16
                                })
                            } else {
                                [15; 4]
                            };
                            let start = translucent.vertices.len() - 4;
                            for (vertex, depth) in
                                translucent.vertices[start..].iter_mut().zip(depths)
                            {
                                let flags = 1 | (u32::from(depth) << 1);
                                vertex[3] = (vertex[3] & !(0x1f << 25)) | ((flags & 0x1f) << 25);
                            }
                        }
                    } else if models::opaque_texture(q.texture) {
                        opaque.quad(q, translation, face, tint, None);
                    } else {
                        cutout.quad(q, translation, face, tint, None);
                    }
                };
                if sample.fluid() >> 4 == 2 {
                    super::fluids::visit(padded, local, &mut emit);
                } else {
                    if sample.model()
                        && crate::block_behavior::catalog::profile(sample.state.kind()).chest
                            == false
                    {
                        models::visit_quads(sample.state, world, &mut emit);
                    }
                    if uneven_water {
                        super::fluids::visit(padded, local, emit);
                    }
                }
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
