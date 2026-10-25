use super::{
    ChunkVoxels, SUN_POSITION, VoxelKind,
    data::{
        CHUNK_HEIGHT, CHUNK_SIZE, FULL_SKY, Layer, SECTION_COUNT, SECTION_HEIGHT, SECTION_VOLUME,
    },
};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, MeshVertexAttribute, VertexFormat},
    prelude::*,
    render::render_resource::PrimitiveTopology,
    shape::Aabb3d,
};
use std::cell::RefCell;

/// Packed terrain and water vertex, decoded by `voxel_vertex.wesl`:
/// - word 0: x, y, z (5 bits each, section-local 0..=16), face direction (3 bits,
///   `axis * 2 + positive`), block kind (8 bits);
/// - word 1: corner sky light and block light (6 bits each, 0..=60); water adds the
///   corner column depth (4 bits) and the lowered surface flag (1 bit) above them.
///
/// UVs, normals, tints and texture layers are derived on the GPU, so a vertex is
/// 8 bytes instead of 56 and meshes do not depend on the texture pack.
pub(crate) const ATTRIBUTE_VOXEL: MeshVertexAttribute =
    MeshVertexAttribute::new("Voxel", 0x766f_7865_6c5f_7631, VertexFormat::Uint32x2);

const _: () = assert!(
    VoxelKind::ALL.len() <= 256,
    "block kinds must fit in 8 bits"
);
const _: () = assert!(
    SECTION_HEIGHT <= 16,
    "section coordinates must fit in 5 bits"
);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Face(VoxelKind, bool, [u16; 4]);

// Nonuniform quads remain one cell wide: repeating the same corner pattern on
// several cells is not equivalent to interpolating once across the merged quad.
#[derive(Clone, Copy, PartialEq, Eq)]
struct LitFace {
    face: Face,
    cell: Option<usize>,
}
impl LitFace {
    fn new(face: Face, cell: usize) -> Self {
        Self {
            cell: (!face.2.iter().all(|&value| value == face.2[0])).then_some(cell),
            face,
        }
    }
}

/// What the mesher needs to know about one cell.
#[derive(Clone, Copy)]
struct Sample {
    kind: VoxelKind,
    light: u8,
    /// Whether the cell darkens the corners of an adjacent face.
    occludes: bool,
}

const ABOVE_WORLD: Sample = Sample {
    kind: VoxelKind::Air,
    light: FULL_SKY,
    occludes: false,
};
const BELOW_WORLD: Sample = Sample {
    kind: VoxelKind::Air,
    light: 0,
    occludes: true,
};

impl Sample {
    #[inline]
    fn new(kind: VoxelKind, light: u8) -> Self {
        Self {
            kind,
            light,
            occludes: kind.is_opaque(),
        }
    }
}

#[derive(Clone)]
struct Column {
    cells: Layer<VoxelKind>,
    light: Layer<u8>,
}

/// A chunk and its eight neighbors, captured in O(1) for a mesh task.
///
/// Missing neighbors read as air (their border faces are drawn) with the chunk's own
/// edge light, so world borders are neither dark nor artificially lit.
#[derive(Clone)]
pub(crate) struct ChunkNeighborhood {
    columns: [Option<Column>; 9],
}

/// Column index of a cell whose y is inside the world; x and z wrap into the column.
#[inline]
fn column_index(x: i32, y: i32, z: i32) -> usize {
    ((x & 15) | (z & 15) << 4 | y << 8) as usize
}

/// 3x3 slot along one horizontal axis: 0 before the chunk, 1 inside, 2 after.
#[inline]
fn slot(value: i32) -> usize {
    if value < 0 {
        0
    } else if value >= CHUNK_SIZE {
        2
    } else {
        1
    }
}

impl ChunkNeighborhood {
    pub(crate) fn new<'a>(
        center: &ChunkVoxels,
        neighbor: impl Fn(IVec2) -> Option<&'a ChunkVoxels>,
    ) -> Self {
        let column = |voxels: &ChunkVoxels| Column {
            cells: voxels.cells().clone(),
            light: voxels.render_light().clone(),
        };
        let columns = std::array::from_fn(|slot| {
            let offset = IVec2::new(slot as i32 % 3 - 1, slot as i32 / 3 - 1);
            if offset == IVec2::ZERO {
                return Some(column(center));
            }
            neighbor(offset).map(column)
        });
        Self { columns }
    }

    /// Bits of the 3x3 slots without a loaded neighbor.
    pub(crate) fn missing_neighbors(&self) -> u16 {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.is_none())
            .fold(0, |mask, (slot, _)| mask | (1 << slot))
    }

    fn center(&self) -> &Column {
        self.columns[4].as_ref().expect("center column")
    }

    fn sample(&self, local: IVec3) -> Sample {
        if local.y >= CHUNK_HEIGHT {
            return ABOVE_WORLD;
        }
        if local.y < 0 {
            return BELOW_WORLD;
        }
        if let Some(column) = &self.columns[slot(local.x) + 3 * slot(local.z)] {
            let index = column_index(local.x, local.y, local.z);
            return Sample::new(column.cells.get(index), column.light.get(index));
        }
        let center = self.center();
        let index = column_index(
            local.x.clamp(0, CHUNK_SIZE - 1),
            local.y,
            local.z.clamp(0, CHUNK_SIZE - 1),
        );
        Sample {
            kind: VoxelKind::Air,
            light: center.light.get(index),
            occludes: center.cells.get(index).is_opaque(),
        }
    }

    /// Whether a section holds opaque blocks and water; uniform sections answer at once.
    fn section_contents(&self, section: usize) -> (bool, bool) {
        let cells = self.center().cells.section(section);
        (
            cells.any(VoxelKind::is_opaque),
            cells.any(|kind| kind == VoxelKind::Water),
        )
    }
}

/// Cells of a padded section row: one border cell on each side.
const PADDED_SIZE: i32 = CHUNK_SIZE + 2;

/// One section plus a one-cell border, copied once so meshing indexes a flat array.
#[derive(Default)]
struct Padded {
    dimensions: [i32; 3],
    samples: Vec<Sample>,
}

impl Padded {
    /// Refills the buffer with a section. Rows are copied 16 cells at a time from the
    /// column that owns them; only the two border cells of a row are sampled one by one.
    fn fill_section(&mut self, neighborhood: &ChunkNeighborhood, section: usize) {
        let base = section as i32 * SECTION_HEIGHT;
        self.dimensions = [CHUNK_SIZE, SECTION_HEIGHT, CHUNK_SIZE];
        self.samples.clear();
        self.samples
            .reserve((PADDED_SIZE * PADDED_SIZE * (SECTION_HEIGHT + 2)) as usize);
        let mut kinds = [VoxelKind::Air; CHUNK_SIZE as usize];
        let mut light = [0; CHUNK_SIZE as usize];
        for y in base - 1..=base + SECTION_HEIGHT {
            for z in -1..=CHUNK_SIZE {
                self.samples.push(neighborhood.sample(IVec3::new(-1, y, z)));
                let column = (0..CHUNK_HEIGHT)
                    .contains(&y)
                    .then(|| neighborhood.columns[1 + 3 * slot(z)].as_ref())
                    .flatten();
                if let Some(column) = column {
                    let section = (y / SECTION_HEIGHT) as usize;
                    let start = column_index(0, y, z) % SECTION_VOLUME;
                    column.cells.section(section).copy_cells(start, &mut kinds);
                    column.light.section(section).copy_cells(start, &mut light);
                    self.samples.extend(
                        kinds
                            .iter()
                            .zip(&light)
                            .map(|(&kind, &light)| Sample::new(kind, light)),
                    );
                } else {
                    self.samples
                        .extend((0..CHUNK_SIZE).map(|x| neighborhood.sample(IVec3::new(x, y, z))));
                }
                self.samples
                    .push(neighborhood.sample(IVec3::new(CHUNK_SIZE, y, z)));
            }
        }
    }

    fn single(kind: VoxelKind) -> Self {
        let mut samples = vec![ABOVE_WORLD; 27];
        samples[13] = Sample::new(kind, FULL_SKY);
        Self {
            dimensions: [1, 1, 1],
            samples,
        }
    }

    fn get(&self, local: IVec3) -> Sample {
        let width = self.dimensions[0] + 2;
        let depth = self.dimensions[2] + 2;
        self.samples[((local.x + 1) + width * ((local.z + 1) + depth * (local.y + 1))) as usize]
    }
}

/// Buffers reused by every section a thread meshes, instead of allocating per section.
#[derive(Default)]
struct Scratch {
    padded: Padded,
    faces: Vec<Option<LitFace>>,
    shadow: Vec<Option<bool>>,
    water: Vec<Option<(LitFace, bool)>>,
    depths: Vec<u8>,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::default();
}

fn face_light(padded: &Padded, outside: [i32; 3], axis: usize) -> [u16; 4] {
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(cu, cv)| {
        let mut sum = [0u32; 2];
        let mut count = 0u32;
        for du in [cu - 1, cu] {
            for dv in [cv - 1, cv] {
                let mut cell = outside;
                cell[u] += du;
                cell[v] += dv;
                let sample = padded.get(IVec3::from_array(cell));
                // Solid neighbors must not darken an exposed surface corner.
                if (du != 0 || dv != 0) && sample.occludes {
                    continue;
                }
                sum[0] += u32::from(sample.light >> 4);
                sum[1] += u32::from(sample.light & 15);
                count += 1;
            }
        }
        let average = |sum: u32| (sum as f32 * 4.0 / count.max(1) as f32).round() as u16;
        average(sum[0]) | (average(sum[1]) << 6)
    })
}

/// Geometry of one render section, in section-local coordinates.
pub(crate) struct SectionMeshes {
    pub(crate) index: usize,
    pub(crate) terrain: Option<TerrainGeometry>,
    pub(crate) water: Option<TerrainGeometry>,
    /// Depth-only terrain for the shadow maps. Only built when sunlight reaches the
    /// section or its border; otherwise its faces are buried beneath other geometry.
    pub(crate) shadow: Option<TerrainGeometry>,
}

/// Terrain or water in the packed [`ATTRIBUTE_VOXEL`] format.
#[derive(Default)]
pub(crate) struct TerrainGeometry {
    vertices: Vec<[u32; 2]>,
    /// A 16³ section has at most 49 152 vertices (a checkerboard): `u16` always fits.
    indices: Vec<u16>,
}

impl TerrainGeometry {
    pub(crate) fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    fn push_quad(
        &mut self,
        corner: [i32; 3],
        du: [i32; 3],
        dv: [i32; 3],
        axis: usize,
        positive: bool,
        kind: VoxelKind,
        word1: [u16; 4],
    ) {
        let add = |a: [i32; 3], b: [i32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let corners = [
            corner,
            add(corner, du),
            add(add(corner, du), dv),
            add(corner, dv),
        ];
        let first = self.vertices.len() as u16;
        let face = (axis as u32 * 2 + u32::from(positive)) << 15 | (kind as u32) << 18;
        self.vertices
            .extend(corners.into_iter().zip(word1).map(|(point, word1)| {
                let [x, y, z] = point.map(|value| value as u32);
                [x | y << 5 | z << 10 | face, u32::from(word1)]
            }));
        let order = if positive {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 2, 1, 0, 3, 2]
        };
        self.indices.extend(order.map(|index| first + index));
    }

    fn position(vertex: [u32; 2]) -> Vec3 {
        Vec3::new(
            (vertex[0] & 31) as f32,
            (vertex[0] >> 5 & 31) as f32,
            (vertex[0] >> 10 & 31) as f32,
        )
    }

    pub(crate) fn into_mesh(self) -> Mesh {
        let (min, max) = self
            .vertices
            .iter()
            .map(|&vertex| Self::position(vertex))
            .fold((Vec3::MAX, Vec3::MIN), |(min, max), position| {
                (min.min(position), max.max(position))
            });
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(ATTRIBUTE_VOXEL, self.vertices)
        .with_inserted_indices(Indices::U16(self.indices));
        // Bevy derives bounds from `ATTRIBUTE_POSITION`, which packed meshes lack.
        mesh.final_aabb = min.cmple(max).all().then(|| Aabb3d::from_min_max(min, max));
        mesh
    }

    #[cfg(feature = "dev")]
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.vertices.len(), self.indices.len() / 3)
    }
}

/// Meshes the requested sections of a chunk; sections without blocks yield no geometry.
pub(crate) fn build_sections(
    neighborhood: &ChunkNeighborhood,
    sections: u32,
) -> Vec<SectionMeshes> {
    SCRATCH.with_borrow_mut(|scratch| {
        let Scratch {
            padded,
            faces,
            shadow,
            water: water_faces,
            depths,
        } = scratch;
        (0..SECTION_COUNT)
            .filter(|section| sections & (1 << section) != 0)
            .map(|index| {
                let (opaque, water) = neighborhood.section_contents(index);
                if !opaque && !water {
                    return SectionMeshes {
                        index,
                        terrain: None,
                        water: None,
                        shadow: None,
                    };
                }
                padded.fill_section(neighborhood, index);
                let casts_shadow = padded.samples.iter().any(|sample| sample.light >> 4 != 0);
                SectionMeshes {
                    index,
                    terrain: opaque
                        .then(|| greedy_mesh(padded, true, faces))
                        .filter(|geometry| !geometry.is_empty()),
                    shadow: (opaque && casts_shadow)
                        .then(|| shadow_mesh(padded, shadow))
                        .filter(|geometry| !geometry.is_empty()),
                    water: water
                        .then(|| {
                            water_mesh(
                                padded,
                                neighborhood,
                                index as i32 * SECTION_HEIGHT,
                                water_faces,
                                depths,
                            )
                        })
                        .flatten(),
                }
            })
            .collect()
    })
}

/// A unit block spanning 0..1 on every axis.
pub(crate) fn build_block_item_mesh(kind: VoxelKind) -> Mesh {
    greedy_mesh(&Padded::single(kind), false, &mut Vec::new()).into_mesh()
}

fn greedy_mesh(padded: &Padded, lit: bool, mask: &mut Vec<Option<LitFace>>) -> TerrainGeometry {
    let dimensions = padded.dimensions;
    let mut geometry = TerrainGeometry::default();
    let largest = dimensions[0].max(dimensions[1]).max(dimensions[2]);
    mask.clear();
    mask.resize((largest * largest) as usize, None);
    for axis in 0..3 {
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let (width, height) = (dimensions[u] as usize, dimensions[v] as usize);

        for layer in -1..dimensions[axis] {
            for j in 0..height {
                for i in 0..width {
                    let mut cell = [0; 3];
                    cell[axis] = layer;
                    cell[u] = i as i32;
                    cell[v] = j as i32;
                    let a = padded.get(IVec3::from_array(cell)).kind;
                    cell[axis] += 1;
                    let b = padded.get(IVec3::from_array(cell)).kind;
                    mask[i + j * width] = match (a.is_opaque(), b.is_opaque()) {
                        (true, false) if layer >= 0 => {
                            let values = if lit {
                                face_light(padded, cell, axis)
                            } else {
                                [60; 4]
                            };
                            Some(LitFace::new(Face(a, true, values), i + j * width))
                        }
                        (false, true) if layer + 1 < dimensions[axis] => {
                            cell[axis] -= 1;
                            let values = if lit {
                                face_light(padded, cell, axis)
                            } else {
                                [60; 4]
                            };
                            Some(LitFace::new(Face(b, false, values), i + j * width))
                        }
                        _ => None,
                    };
                }
            }

            consume_mask(
                &mut mask[..width * height],
                width,
                height,
                |i, j, quad_width, quad_height, face| {
                    let (corner, du, dv) = quad(axis, layer, i, j, quad_width, quad_height);
                    let Face(kind, positive, light) = face.face;
                    geometry.push_quad(corner, du, dv, axis, positive, kind, light);
                },
            );
        }
    }
    geometry
}

/// Corner, width and height vectors of a mask rectangle on the plane after `layer`.
fn quad(
    axis: usize,
    layer: i32,
    i: usize,
    j: usize,
    width: usize,
    height: usize,
) -> ([i32; 3], [i32; 3], [i32; 3]) {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut corner = [0; 3];
    corner[axis] = layer + 1;
    corner[u] = i as i32;
    corner[v] = j as i32;
    let mut du = [0; 3];
    let mut dv = [0; 3];
    du[u] = width as i32;
    dv[v] = height as i32;
    (corner, du, dv)
}

/// Whether the shadow maps need faces of this orientation.
///
/// The shadow pass rasterizes both sides, but a face turned away from the sun always
/// lies behind a sunward face of the same closed opaque surface: dropping it leaves
/// the depth seen from the light unchanged. Remesh everything if the sun ever moves.
fn faces_sun(axis: usize, positive: bool) -> bool {
    let toward = SUN_POSITION[axis];
    toward == 0.0 || (toward > 0.0) == positive
}

/// Depth-only terrain: faces merge regardless of block, texture or light, and only
/// those facing the sun are kept.
fn shadow_mesh(padded: &Padded, mask: &mut Vec<Option<bool>>) -> TerrainGeometry {
    let dimensions = padded.dimensions;
    let mut geometry = TerrainGeometry::default();
    let largest = dimensions[0].max(dimensions[1]).max(dimensions[2]);
    mask.clear();
    mask.resize((largest * largest) as usize, None);
    for axis in 0..3 {
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let (width, height) = (dimensions[u] as usize, dimensions[v] as usize);
        let (keep_positive, keep_negative) = (faces_sun(axis, true), faces_sun(axis, false));

        for layer in -1..dimensions[axis] {
            for j in 0..height {
                for i in 0..width {
                    let mut cell = [0; 3];
                    cell[axis] = layer;
                    cell[u] = i as i32;
                    cell[v] = j as i32;
                    let a = padded.get(IVec3::from_array(cell)).kind.is_opaque();
                    cell[axis] += 1;
                    let b = padded.get(IVec3::from_array(cell)).kind.is_opaque();
                    mask[i + j * width] = match (a, b) {
                        (true, false) if keep_positive && layer >= 0 => Some(true),
                        (false, true) if keep_negative && layer + 1 < dimensions[axis] => {
                            Some(false)
                        }
                        _ => None,
                    };
                }
            }

            consume_mask(
                &mut mask[..width * height],
                width,
                height,
                |i, j, quad_width, quad_height, positive| {
                    let (corner, du, dv) = quad(axis, layer, i, j, quad_width, quad_height);
                    geometry.push_quad(corner, du, dv, axis, positive, VoxelKind::Air, [0; 4]);
                },
            );
        }
    }
    geometry
}

/// Water columns deeper than this read as fully deep in the shader.
const WATER_DEPTH_CAP: usize = 8;
/// Depth is packed in the bits above the 12 bits of corner light, as 0..=15.
const WATER_DEPTH_SHIFT: u16 = 12;
const WATER_DEPTH_MAX: u16 = 15;
/// Marks corners at the lowered surface level, 2/16 below the block top
/// (`water_material.wesl` applies the drop).
const WATER_LOWERED: u32 = 1 << 16;

/// Water cells stacked downward from `top` (inclusive), capped at [`WATER_DEPTH_CAP`].
fn water_depth(neighborhood: &ChunkNeighborhood, top: IVec3) -> u8 {
    (0..WATER_DEPTH_CAP as i32)
        .take_while(|&below| neighborhood.sample(top - IVec3::Y * below).kind == VoxelKind::Water)
        .count() as u8
}

/// Corner depths of a surface face, averaged over the four columns around each
/// corner so shores fade smoothly into shallow water.
fn surface_depth(depths: &[u8], stride: i32, x: i32, z: i32) -> [u16; 4] {
    // Axis 1 corners walk u = z then v = x, matching `face_light`.
    [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(cz, cx)| {
        let mut sum = 0u32;
        for dz in [cz - 1, cz] {
            for dx in [cx - 1, cx] {
                sum += u32::from(depths[((x + dx + 1) + stride * (z + dz + 1)) as usize]);
            }
        }
        (sum as f32 * f32::from(WATER_DEPTH_MAX) / (4 * WATER_DEPTH_CAP) as f32).round() as u16
    })
}

fn water_mesh(
    padded: &Padded,
    neighborhood: &ChunkNeighborhood,
    base: i32,
    mask: &mut Vec<Option<(LitFace, bool)>>,
    depths: &mut Vec<u8>,
) -> Option<TerrainGeometry> {
    let dimensions = padded.dimensions;
    let mut geometry = TerrainGeometry::default();
    let stride = dimensions[0] + 2;
    for axis in 0..3 {
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let (width, height) = (dimensions[u] as usize, dimensions[v] as usize);
        mask.clear();
        mask.resize(width * height, None);
        for layer in -1..dimensions[axis] {
            // Column depths of this layer, computed once and only if it has a surface.
            depths.clear();
            for j in 0..height {
                for i in 0..width {
                    let mut cell = [0; 3];
                    cell[axis] = layer;
                    cell[u] = i as i32;
                    cell[v] = j as i32;
                    let a = padded.get(IVec3::from_array(cell)).kind;
                    cell[axis] += 1;
                    let b = padded.get(IVec3::from_array(cell)).kind;
                    // Only real air/fluid interfaces. No water plane inside a solid
                    // ceiling, no interior faces, and no faces owned by a neighbor.
                    let (positive, water_cell, outside) = match (a, b) {
                        (VoxelKind::Water, VoxelKind::Air) if layer >= 0 => {
                            let mut water_cell = cell;
                            water_cell[axis] -= 1;
                            (true, water_cell, cell)
                        }
                        (VoxelKind::Air, VoxelKind::Water) if layer + 1 < dimensions[axis] => {
                            let mut outside = cell;
                            outside[axis] -= 1;
                            (false, cell, outside)
                        }
                        _ => {
                            mask[i + j * width] = None;
                            continue;
                        }
                    };
                    let water_cell = IVec3::from_array(water_cell);
                    let mut values = face_light(padded, outside, axis);
                    let surface = axis == 1 && positive;
                    let depth = if surface {
                        if depths.is_empty() {
                            depths.extend(
                                (-1..=dimensions[2])
                                    .flat_map(|z| (-1..=dimensions[0]).map(move |x| (x, z)))
                                    .map(|(x, z)| {
                                        water_depth(neighborhood, IVec3::new(x, base + layer, z))
                                    }),
                            );
                        }
                        surface_depth(depths, stride, water_cell.x, water_cell.z)
                    } else {
                        [WATER_DEPTH_MAX; 4]
                    };
                    for (value, depth) in values.iter_mut().zip(depth) {
                        *value |= depth << WATER_DEPTH_SHIFT;
                    }
                    // Top faces and the upper rim of exposed sides sit at the lowered level.
                    let lowered = surface
                        || (axis != 1
                            && padded.get(water_cell + IVec3::Y).kind != VoxelKind::Water);
                    mask[i + j * width] = Some((
                        LitFace::new(Face(VoxelKind::Water, positive, values), i + j * width),
                        lowered,
                    ));
                }
            }
            consume_mask(mask, width, height, |i, j, w, h, (face, lowered)| {
                let (corner, du, dv) = quad(axis, layer, i, j, w, h);
                let Face(kind, positive, values) = face.face;
                geometry.push_quad(corner, du, dv, axis, positive, kind, values);
                if lowered {
                    // A lowered side never merges vertically: the cell above it is
                    // not water, so the next cell up has no side face to join.
                    let top = (corner[1] + du[1] + dv[1]) as u32;
                    let start = geometry.vertices.len() - 4;
                    for vertex in &mut geometry.vertices[start..] {
                        if TerrainGeometry::position(*vertex).y as u32 == top {
                            vertex[1] |= WATER_LOWERED;
                        }
                    }
                }
            });
        }
    }
    (!geometry.is_empty()).then_some(geometry)
}

fn consume_mask<T: Copy + Eq>(
    mask: &mut [Option<T>],
    width: usize,
    height: usize,
    mut emit: impl FnMut(usize, usize, usize, usize, T),
) {
    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let Some(value) = mask[x + y * width] else {
                x += 1;
                continue;
            };
            let mut quad_width = 1;
            while x + quad_width < width && mask[x + quad_width + y * width] == Some(value) {
                quad_width += 1;
            }
            let mut quad_height = 1;
            'grow: while y + quad_height < height {
                for offset in 0..quad_width {
                    if mask[x + offset + (y + quad_height) * width] != Some(value) {
                        break 'grow;
                    }
                }
                quad_height += 1;
            }
            emit(x, y, quad_width, quad_height, value);
            for row in 0..quad_height {
                for column in 0..quad_width {
                    mask[x + column + (y + row) * width] = None;
                }
            }
            x += quad_width;
        }
        y += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(center: &ChunkVoxels, neighbor: Option<(IVec2, &ChunkVoxels)>) -> SectionMeshes {
        let neighborhood = ChunkNeighborhood::new(center, |offset| {
            neighbor.and_then(|(position, voxels)| (position == offset).then_some(voxels))
        });
        build_sections(&neighborhood, 1).remove(0)
    }

    #[test]
    fn light_gradients_are_not_collapsed_across_greedy_faces() {
        let uniform = Face(VoxelKind::Stone, true, [60; 4]);
        let gradient = Face(VoxelKind::Stone, true, [0, 20, 40, 60]);
        for (face, expected) in [(uniform, 1), (gradient, 2)] {
            let mut mask = vec![Some(LitFace::new(face, 0)), Some(LitFace::new(face, 1))];
            let mut count = 0;
            consume_mask(&mut mask, 2, 1, |_, _, _, _, _| count += 1);
            assert_eq!(count, expected);
        }
    }

    /// An empty column with these blocks.
    fn chunk(blocks: &[(IVec3, VoxelKind)]) -> ChunkVoxels {
        let mut chunk = ChunkVoxels::filled(VoxelKind::Air);
        for &(local, kind) in blocks {
            assert!(chunk.set(local, kind));
        }
        chunk
    }

    #[test]
    fn neighbors_never_emit_neighbor_owned_faces() {
        for direction in [IVec2::NEG_X, IVec2::X, IVec2::NEG_Y, IVec2::Y] {
            let edge = |value: i32| match value {
                -1 => 0,
                1 => CHUNK_SIZE - 1,
                _ => 5,
            };
            let cell = IVec3::new(edge(direction.x), 5, edge(direction.y));
            let across = IVec3::new((cell.x + direction.x) & 15, 5, (cell.z + direction.y) & 15);
            let neighbor = chunk(&[(across, VoxelKind::Stone)]);
            let empty = chunk(&[]);
            assert!(mesh(&empty, Some((direction, &neighbor))).terrain.is_none());
            let center = chunk(&[(cell, VoxelKind::Stone)]);
            let solid = mesh(&center, Some((direction, &neighbor))).terrain.unwrap();
            assert_eq!(solid.indices.len(), 5 * 6);
            assert_eq!(mesh(&center, None).terrain.unwrap().indices.len(), 6 * 6);
        }
    }

    #[test]
    fn neighbor_light_reaches_border_face_corners() {
        let center = ChunkVoxels::filled(VoxelKind::Stone).with_light(0);
        let lit = ChunkVoxels::filled(VoxelKind::Air).with_light(0x37);
        let neighborhood =
            ChunkNeighborhood::new(&center, |offset| (offset == IVec2::NEG_X).then_some(&lit));
        let sample = neighborhood.sample(IVec3::new(-1, 0, 0));
        assert_eq!(sample.light, 0x37);
        assert!(!sample.occludes);
        assert_eq!(
            neighborhood.sample(IVec3::new(0, CHUNK_HEIGHT, 0)).light,
            FULL_SKY
        );
        assert!(neighborhood.sample(IVec3::NEG_Y).occludes);
        // A missing neighbor repeats the chunk's own edge.
        assert!(neighborhood.sample(IVec3::new(CHUNK_SIZE, 0, 0)).occludes);
    }

    #[test]
    fn padded_rows_match_cell_by_cell_sampling() {
        let center = chunk(&[
            (IVec3::new(0, 15, 0), VoxelKind::Stone),
            (IVec3::new(15, 16, 7), VoxelKind::Water),
            (IVec3::new(3, 17, 15), VoxelKind::Dirt),
        ]);
        let neighbor = chunk(&[(IVec3::new(3, 16, 0), VoxelKind::Sand)]);
        let neighborhood =
            ChunkNeighborhood::new(&center, |offset| (offset == IVec2::Y).then_some(&neighbor));
        let mut padded = Padded::default();
        for section in [0, 1, SECTION_COUNT - 1] {
            padded.fill_section(&neighborhood, section);
            let base = section as i32 * SECTION_HEIGHT;
            for y in -1..=SECTION_HEIGHT {
                for z in -1..=CHUNK_SIZE {
                    for x in -1..=CHUNK_SIZE {
                        let expected = neighborhood.sample(IVec3::new(x, base + y, z));
                        let actual = padded.get(IVec3::new(x, y, z));
                        assert_eq!(
                            (actual.kind, actual.light, actual.occludes),
                            (expected.kind, expected.light, expected.occludes)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn water_has_side_faces_and_no_faces_inside_solid_ceilings() {
        let cell = IVec3::new(5, 5, 5);
        let mut water = chunk(&[(cell, VoxelKind::Water)]);
        let has_face = |geometry: &TerrainGeometry, direction: u32| {
            geometry
                .vertices
                .iter()
                .any(|vertex| vertex[0] >> 15 & 7 == direction)
        };
        let geometry = mesh(&water, None).water.unwrap();
        assert_eq!(geometry.indices.len(), 6 * 6);
        assert!(has_face(&geometry, 1));
        assert!(has_face(&geometry, 3));
        // The surface and the upper rim of the sides sit at the lowered level.
        assert!(
            geometry
                .vertices
                .iter()
                .all(|vertex| (vertex[1] & WATER_LOWERED != 0)
                    == (TerrainGeometry::position(*vertex).y == 6.0))
        );
        water.set(cell + IVec3::Y, VoxelKind::Stone);
        let geometry = mesh(&water, None).water.unwrap();
        assert_eq!(geometry.indices.len(), 5 * 6);
        assert!(!has_face(&geometry, 3));
    }

    #[test]
    fn sections_are_meshed_independently_and_empty_ones_are_skipped() {
        let chunk = chunk(&[(IVec3::new(5, 40, 5), VoxelKind::Stone)]);
        let neighborhood = ChunkNeighborhood::new(&chunk, |_| None);
        let sections = build_sections(&neighborhood, 0b111);
        assert_eq!(sections.len(), 3);
        assert!(sections[0].terrain.is_none() && sections[1].terrain.is_none());
        assert_eq!(sections[2].terrain.as_ref().unwrap().indices.len(), 6 * 6);
        // Positions are local to the section.
        assert!(
            sections[2]
                .terrain
                .as_ref()
                .unwrap()
                .vertices
                .iter()
                .all(|&vertex| TerrainGeometry::position(vertex).y <= 9.0)
        );
    }

    #[test]
    fn shadow_meshes_merge_blocks_and_keep_only_sunward_faces() {
        // Two different blocks side by side: the lit mesh keeps them apart, the
        // shadow mesh merges them and drops the faces turned away from the sun.
        let chunk = chunk(&[
            (IVec3::new(5, 5, 5), VoxelKind::Stone),
            (IVec3::new(6, 5, 5), VoxelKind::Dirt),
        ]);
        let section = build_sections(&ChunkNeighborhood::new(&chunk, |_| None), 1).remove(0);
        let shadow = section.shadow.unwrap();
        let sunward = (0..3).filter(|&axis| faces_sun(axis, true)).count()
            + (0..3).filter(|&axis| faces_sun(axis, false)).count();
        assert_eq!(sunward, 3);
        assert_eq!(shadow.indices.len(), sunward * 6);
        assert!(section.terrain.unwrap().indices.len() > 6 * 6);
        for vertex in &shadow.vertices {
            let direction = vertex[0] >> 15 & 7;
            assert!(faces_sun((direction / 2) as usize, direction % 2 == 1));
        }
    }

    #[test]
    fn packed_vertices_round_trip_positions_and_light() {
        let mut geometry = TerrainGeometry::default();
        geometry.push_quad(
            [16, 0, 3],
            [0, 16, 0],
            [0, 0, 2],
            0,
            true,
            VoxelKind::Grass,
            [0, 60 | 15 << 6, 7, 60],
        );
        let corners: Vec<_> = geometry
            .vertices
            .iter()
            .map(|&vertex| TerrainGeometry::position(vertex))
            .collect();
        assert_eq!(corners[0], Vec3::new(16.0, 0.0, 3.0));
        assert_eq!(corners[2], Vec3::new(16.0, 16.0, 5.0));
        assert_eq!(geometry.vertices[1][1], 60 | 15 << 6);
        assert_eq!(geometry.vertices[0][0] >> 18 & 255, VoxelKind::Grass as u32);
        assert_eq!(geometry.vertices[0][0] >> 15 & 7, 1);
    }
}
