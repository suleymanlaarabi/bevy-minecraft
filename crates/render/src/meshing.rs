use super::models::{CellInfo, cell};
use crate::source::RenderSource;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, MeshVertexAttribute, VertexFormat},
    prelude::*,
    render::render_resource::PrimitiveTopology,
    shape::Aabb3d,
};
use hollow_core::coordinates::{CHUNK_SIZE, FULL_SKY, SECTION_HEIGHT, SECTION_VOLUME};
use hollow_core::sections::Layer;
use hollow_core::{StateId, VoxelCell};
use hollow_engine::chunk::ChunkVoxels;
use std::cell::RefCell;

/// Packed terrain and water vertex, decoded by `voxel_vertex.wesl`:
/// - word 0: x, y, z (5 bits each, section-local 0..=16), face direction (3 bits,
///   `axis * 2 + positive`), cube appearance (14 bits, see `models::Appearance`);
/// - word 1: corner sky light and block light (6 bits each, 0..=60); water adds the
///   corner column depth (4 bits) and the lowered surface flag (1 bit) above them.
///
/// UVs, normals, tints and texture layers are derived on the GPU, so a vertex is
/// 8 bytes instead of 56 and meshes do not depend on the texture pack.
pub const ATTRIBUTE_VOXEL: MeshVertexAttribute =
    MeshVertexAttribute::new("Voxel", 0x766f_7865_6c5f_7631, VertexFormat::Uint32x2);

const _: () = assert!(
    CHUNK_SIZE <= 16 && SECTION_HEIGHT <= 16,
    "section coordinates must fit in 5 bits"
);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Face(u16, bool, [u16; 4]);

// Nonuniform quads remain one cell wide: repeating the same corner pattern on
// several cells is not equivalent to interpolating once across the merged quad.
#[derive(Clone, Copy, PartialEq, Eq)]
struct LitFace {
    face: Face,
    cell: Option<usize>,
}
impl LitFace {
    /// `per_cell` keeps even uniform faces from merging with their neighbors.
    fn new(face: Face, cell: usize, per_cell: bool) -> Self {
        Self {
            cell: (per_cell || !face.2.iter().all(|&value| value == face.2[0])).then_some(cell),
            face,
        }
    }
}

/// What the mesher needs to know about one cell: its state and one table read.
#[derive(Clone, Copy)]
pub struct Sample {
    pub state: StateId,
    info: CellInfo,
    pub light: u8,
}

const ABOVE_WORLD: Sample = Sample {
    state: StateId(0),
    info: CellInfo {
        appearance: 0,
        flags: cell::AIR,
        fluid: 0,
    },
    light: FULL_SKY,
};
const BELOW_WORLD: Sample = Sample {
    state: StateId(0),
    info: CellInfo {
        appearance: 0,
        flags: cell::AIR | cell::OCCLUDES,
        fluid: 0,
    },
    light: 0,
};

impl Sample {
    #[inline]
    fn state<C: RenderSource>(source: &C, state: StateId, light: u8) -> Self {
        Self {
            state,
            info: source.cell_info(state),
            light,
        }
    }
    #[inline]
    fn has(&self, flag: u8) -> bool {
        self.info.flags & flag != 0
    }
    /// Opaque greedy cube.
    #[inline]
    pub fn cube(&self) -> bool {
        self.has(cell::CUBE)
    }
    #[inline]
    pub fn ocean_surface(&self) -> bool {
        self.has(cell::OCEAN_SURFACE)
    }
    #[inline]
    pub fn fluid(&self) -> u32 {
        self.info.fluid
    }
    pub fn fluid_id(&self) -> hollow_core::FluidId {
        hollow_core::FluidId(((self.fluid() >> 8) & 0xffff) as u16)
    }
    pub fn fluid_amount(&self) -> u8 {
        self.fluid() as u8
    }
    pub fn fluid_falling(&self) -> bool {
        self.fluid() & (1 << 24) != 0
    }
    #[inline]
    pub fn model(&self) -> bool {
        self.has(cell::MODEL)
    }
    /// Whether the cell hides adjacent faces and darkens their corners.
    #[inline]
    pub fn occludes(&self) -> bool {
        self.has(cell::OCCLUDES)
    }
    /// Whether a neighboring water face is enclosed by this solid material.
    #[inline]
    pub fn hides_fluid(&self) -> bool {
        self.has(cell::OCCLUDES | cell::FLUID_OCCLUDES)
    }
    #[inline]
    pub fn fluid_occluder(&self) -> bool {
        self.has(cell::FLUID_OCCLUDER)
    }
}

#[derive(Clone)]
struct Column<T: VoxelCell> {
    states: Layer<T>,
    light: Layer<u8>,
}

/// A chunk and its eight neighbors, captured in O(1) for a mesh task.
///
/// Missing neighbors read as air (their border faces are drawn) with the chunk's own
/// edge light, so world borders are neither dark nor artificially lit.
#[derive(Clone)]
pub struct ChunkNeighborhood<C: RenderSource> {
    pub source: C,
    pub layout: hollow_core::coordinates::ColumnLayout,
    pub origin: IVec3,
    columns: [Option<Column<C::Cell>>; 9],
    cached_connectivity: Box<[Option<u64>]>,
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

impl<C: RenderSource> ChunkNeighborhood<C> {
    pub fn new<'a>(
        center: &ChunkVoxels<C::Cell>,
        source: C,
        layout: hollow_core::coordinates::ColumnLayout,
        neighbor: impl Fn(IVec2) -> Option<&'a ChunkVoxels<C::Cell>>,
    ) -> Self {
        let column = |voxels: &ChunkVoxels<C::Cell>| Column {
            states: voxels.states.clone(),
            light: voxels.render_light().clone(),
        };
        let columns = std::array::from_fn(|slot| {
            let offset = IVec2::new(slot as i32 % 3 - 1, slot as i32 / 3 - 1);
            if offset == IVec2::ZERO {
                return Some(column(center));
            }
            neighbor(offset).map(column)
        });
        Self {
            columns,
            source,
            layout,
            cached_connectivity: vec![None; center.states.section_count()].into_boxed_slice(),
            origin: IVec3::ZERO,
        }
    }

    pub fn at(mut self, chunk: IVec2) -> Self {
        self.origin = IVec3::new(
            chunk.x * CHUNK_SIZE,
            self.layout.min_y(),
            chunk.y * CHUNK_SIZE,
        );
        self
    }

    pub fn reuse_connectivity(mut self, values: impl Iterator<Item = u64>, built: u32) -> Self {
        for (section, (slot, value)) in self.cached_connectivity.iter_mut().zip(values).enumerate()
        {
            *slot = (built & (1 << section) != 0).then_some(value);
        }
        self
    }

    /// Bits of the 3x3 slots without a loaded neighbor.
    pub fn missing_neighbors(&self) -> u16 {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.is_none())
            .fold(0, |mask, (slot, _)| mask | (1 << slot))
    }

    fn center(&self) -> &Column<C::Cell> {
        self.columns[4].as_ref().expect("center column")
    }

    fn sample(&self, local: IVec3) -> Sample {
        if local.y >= self.layout.height() {
            return ABOVE_WORLD;
        }
        if local.y < 0 {
            return BELOW_WORLD;
        }
        if let Some(column) = &self.columns[slot(local.x) + 3 * slot(local.z)] {
            let index = column_index(local.x, local.y, local.z);
            let state = column.states.get(index);
            return Sample::state(&self.source, StateId(state.id()), column.light.get(index));
        }
        let center = self.center();
        let index = column_index(
            local.x.clamp(0, CHUNK_SIZE - 1),
            local.y,
            local.z.clamp(0, CHUNK_SIZE - 1),
        );
        Sample {
            light: center.light.get(index),
            ..if center.states.get(index).is_opaque(self.source.content()) {
                BELOW_WORLD
            } else {
                ABOVE_WORLD
            }
        }
    }

    /// Whether a section holds opaque blocks and water; uniform sections answer at once.
    fn section_contents(&self, section: usize) -> (bool, bool) {
        let states = self.center().states.section(section);
        (
            states.any(|state| {
                let info = self.source.cell_info(StateId(state.id()));
                info.flags & (cell::CUBE | cell::CUTOUT | cell::MODEL) != 0
                    || (info.fluid != 0 && info.flags & cell::OCEAN_SURFACE == 0)
            }),
            states.any(|state| {
                self.source.cell_info(StateId(state.id())).flags & cell::OCEAN_SURFACE != 0
            }),
        )
    }
}

/// Cells of a padded section row: one border cell on each side.
const PADDED_SIZE: i32 = CHUNK_SIZE + 2;
const ROW_BITS: usize = PADDED_SIZE as usize;
const AXIS_ROWS: usize = ROW_BITS * ROW_BITS;

struct SectionSummary {
    flags: u8,
    fluids: bool,
    sky: bool,
    enclosed: bool,
    opaque: [u64; 64],
}

impl Default for SectionSummary {
    fn default() -> Self {
        Self {
            flags: 0,
            fluids: false,
            sky: false,
            enclosed: true,
            opaque: [0; 64],
        }
    }
}

/// One section plus a one-cell border, copied once so meshing indexes a flat array.
#[derive(Default)]
pub struct Padded {
    dimensions: [i32; 3],
    samples: Vec<Sample>,
    summary: SectionSummary,
    // Three orientations, with cube/cutout/occluder bits packed in each row.
    // Shared by terrain and shadow meshing; retained in the thread's scratch buffer.
    rows: Vec<u64>,
    /// Includes the halo: a neighboring partial block can cover a fluid face.
    pub fluid_occluders: bool,
}

impl Padded {
    /// Refills the buffer with a section. Rows are copied 16 cells at a time from the
    /// column that owns them; only the two border cells of a row are sampled one by one.
    fn fill_section<C: RenderSource>(
        &mut self,
        neighborhood: &ChunkNeighborhood<C>,
        section: usize,
    ) {
        let base = section as i32 * SECTION_HEIGHT;
        self.dimensions = [CHUNK_SIZE, SECTION_HEIGHT, CHUNK_SIZE];
        self.samples.clear();
        self.samples
            .reserve((PADDED_SIZE * PADDED_SIZE * (SECTION_HEIGHT + 2)) as usize);
        let mut light = [0; CHUNK_SIZE as usize];
        let mut states = [C::Cell::air(); CHUNK_SIZE as usize];
        let build_connectivity = neighborhood.cached_connectivity[section].is_none();
        let mut summary = SectionSummary::default();
        self.fluid_occluders = false;
        for y in base - 1..=base + SECTION_HEIGHT {
            for z in -1..=CHUNK_SIZE {
                self.samples.push(neighborhood.sample(IVec3::new(-1, y, z)));
                let column = (0..neighborhood.layout.height())
                    .contains(&y)
                    .then(|| neighborhood.columns[1 + 3 * slot(z)].as_ref())
                    .flatten();
                if let Some(column) = column {
                    let section = (y / SECTION_HEIGHT) as usize;
                    let start = column_index(0, y, z) % SECTION_VOLUME;
                    column.light.section(section).copy_cells(start, &mut light);
                    column
                        .states
                        .section(section)
                        .copy_cells(start, &mut states);
                    self.samples
                        .extend(states.iter().zip(&light).map(|(&id, &light)| {
                            Sample::state(&neighborhood.source, StateId(id.id()), light)
                        }));
                } else {
                    self.samples
                        .extend((0..CHUNK_SIZE).map(|x| neighborhood.sample(IVec3::new(x, y, z))));
                }
                self.samples
                    .push(neighborhood.sample(IVec3::new(CHUNK_SIZE, y, z)));
                // Summarize the row while it is hot, using the known row
                // coordinates rather than dividing every padded cell index.
                let interior =
                    (base..base + SECTION_HEIGHT).contains(&y) && (0..CHUNK_SIZE).contains(&z);
                let row = &self.samples[self.samples.len() - PADDED_SIZE as usize..];
                for (x, sample) in row.iter().enumerate() {
                    summary.sky |= sample.light >> 4 != 0;
                    summary.enclosed &= sample.occludes();
                    self.fluid_occluders |= sample.fluid_occluder();
                    if interior && (1..=CHUNK_SIZE as usize).contains(&x) {
                        summary.flags |= sample.info.flags;
                        summary.fluids |= sample.fluid() != 0;
                        if build_connectivity && sample.cube() && sample.occludes() {
                            let cell = x - 1 + z as usize * 16 + (y - base) as usize * 256;
                            summary.opaque[cell / 64] |= 1 << (cell % 64);
                        }
                    }
                }
            }
        }
        self.summary = summary;
    }

    /// Build only for sections with exposed cube geometry; enclosed and model-only
    /// sections keep the cheap fill path. All material/shadow passes reuse these rows.
    fn prepare_rows(&mut self) {
        self.rows.resize(3 * AXIS_ROWS, 0);
        self.rows.fill(0);
        for (index, row) in self.samples.chunks_exact(ROW_BITS).enumerate() {
            let (py, pz) = (index / ROW_BITS, index % ROW_BITS);
            let mut row_bits = 0;
            for (x, sample) in row.iter().enumerate() {
                // Row bits run along u=(axis+1)%3; row indices are normal+v*18.
                // Each 18-bit lane includes both border cells.
                let flags = u64::from(sample.cube())
                    | (u64::from(sample.has(cell::CUTOUT)) << ROW_BITS)
                    | (u64::from(sample.occludes()) << (2 * ROW_BITS));
                if flags != 0 {
                    self.rows[x + ROW_BITS * pz] |= flags << py;
                    self.rows[AXIS_ROWS + py + ROW_BITS * x] |= flags << pz;
                    row_bits |= flags << x;
                }
            }
            self.rows[2 * AXIS_ROWS + pz + ROW_BITS * py] = row_bits;
        }
    }

    #[inline]
    pub fn get(&self, local: IVec3) -> Sample {
        self.samples
            [((local.x + 1) + PADDED_SIZE * ((local.z + 1) + PADDED_SIZE * (local.y + 1))) as usize]
    }
}

/// Buffers reused by every section a thread meshes, instead of allocating per section.
struct Scratch {
    padded: Padded,
    faces: Vec<Option<LitFace>>,
    water: Vec<Option<(LitFace, bool)>>,
    depths: Vec<u8>,
    model_depths: Vec<u8>,
    visited: [u64; 64],
    flood_stack: Vec<u16>,
}

impl Default for Scratch {
    fn default() -> Self {
        Self {
            padded: default(),
            faces: Vec::new(),
            water: Vec::new(),
            depths: Vec::new(),
            model_depths: Vec::new(),
            visited: [0; 64],
            flood_stack: Vec::new(),
        }
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::default();
}

pub fn face_light(padded: &Padded, outside: [i32; 3], axis: usize) -> [u16; 4] {
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
                if (du != 0 || dv != 0) && sample.occludes() {
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
#[derive(Default)]
pub struct SectionMeshes {
    pub connectivity: u64,
    /// Alpha-tested greedy cubes (leaves), drawn and shadowed with discard.
    pub cutout: Option<TerrainGeometry<u32>>,
    pub models: Option<super::model_mesh::ModelGeometry>,
    pub models_cutout: Option<super::model_mesh::ModelGeometry>,
    pub translucent: Option<super::model_mesh::ModelGeometry>,
    pub index: usize,
    pub terrain: Option<TerrainGeometry>,
    /// Depth-only terrain for the shadow maps. Only built when sunlight reaches the
    /// section or its border; otherwise its faces are buried beneath other geometry.
    pub shadow: Option<TerrainGeometry>,
}

/// Terrain or water in the packed [`ATTRIBUTE_VOXEL`] format.
pub struct TerrainGeometry<I: TerrainIndex = u16> {
    pub vertices: Vec<[u32; 2]>,
    pub indices: Vec<I>,
}

// Packed section coordinates span 0..=16. Each grid line stores its existing
// vertices in a bitset, so finding T-junctions requires no spatial searches.
const EDGE_GRID: usize = 17;
const _: () = assert!(CHUNK_SIZE < EDGE_GRID as i32 && SECTION_HEIGHT < EDGE_GRID as i32);

struct EdgeVertices {
    lines: [[u32; EDGE_GRID * EDGE_GRID]; 3],
}

impl EdgeVertices {
    fn new(vertices: impl Iterator<Item = [u32; 2]>, padded: &Padded) -> Self {
        let mut edges = Self {
            lines: [[0; EDGE_GRID * EDGE_GRID]; 3],
        };
        for vertex in vertices {
            edges.insert(grid_position(vertex));
        }
        // Neighbor sections mesh independently, so a long edge on a shared plane
        // cannot know where the other side's corners fall. Both sides split at
        // every point where faces of two sections meet: the eight cells around a
        // point decide it, and every section sharing the point holds them all.
        let dimensions = padded.dimensions.map(|size| size as u32);
        for y in 0..=dimensions[1] {
            for z in 0..=dimensions[2] {
                let interior_row =
                    (1..dimensions[1]).contains(&y) && (1..dimensions[2]).contains(&z);
                // Interior rows only touch the boundary at their two ends.
                let step = if interior_row {
                    dimensions[0] as usize
                } else {
                    1
                };
                for x in (0..=dimensions[0]).step_by(step) {
                    let point = [x, y, z];
                    if shared_point(padded, point) {
                        edges.insert(point);
                    }
                }
            }
        }
        edges
    }

    fn insert(&mut self, point: [u32; 3]) {
        for axis in 0..3 {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            self.lines[axis][point[u] as usize + EDGE_GRID * point[v] as usize] |= 1 << point[axis];
        }
    }

    fn splits(&self, a: [u32; 3], b: [u32; 3], axis: usize) -> u32 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let points = self.lines[axis][a[u] as usize + EDGE_GRID * a[v] as usize];
        let (low, high) = (a[axis].min(b[axis]), a[axis].max(b[axis]));
        points & ((1 << high) - 1) & !((1 << (low + 1)) - 1)
    }
}

/// Whether greedy faces of more than one section touch a section-boundary point.
/// Every face containing the point lies between two of the eight cells around it,
/// and belongs to the section of its solid cell.
fn shared_point(padded: &Padded, point: [u32; 3]) -> bool {
    let mut solid = 0u8;
    let mut occluders = 0u8;
    // Each X-oriented row supplies two adjacent corners. The four Y/Z rows
    // already encode exactly the cube/cutout/occlusion predicates used here.
    for y in 0..2 {
        for z in 0..2 {
            let row = padded.rows
                [2 * AXIS_ROWS + point[2] as usize + z + ROW_BITS * (point[1] as usize + y)];
            let shift = 2 * y + 4 * z;
            solid |= ((((row | (row >> ROW_BITS)) >> point[0]) & 3) as u8) << shift;
            occluders |= (((row >> (2 * ROW_BITS + point[0] as usize)) & 3) as u8) << shift;
        }
    }
    // Corner bits are X/Y/Z. Swapping each axis pairs every corner with its
    // neighbor; only corners blocked along all three axes lack an exposed face.
    let x = ((occluders & 0x55) << 1) | ((occluders & 0xaa) >> 1);
    let y = ((occluders & 0x33) << 2) | ((occluders & 0xcc) >> 2);
    let z = ((occluders & 0x0f) << 4) | ((occluders & 0xf0) >> 4);
    let exposed = solid & !(x & y & z);
    let first = exposed.trailing_zeros();
    if first == 8 {
        return false;
    }
    let mut owner = 0xffu8;
    for (axis, low) in [0x55u8, 0x33, 0x0f].into_iter().enumerate() {
        if point[axis] == 0 || point[axis] == padded.dimensions[axis] as u32 {
            owner &= if first & (1 << axis) == 0 { low } else { !low };
        }
    }
    exposed & !owner != 0
}

fn grid_position(vertex: [u32; 2]) -> [u32; 3] {
    [vertex[0] & 31, vertex[0] >> 5 & 31, vertex[0] >> 10 & 31]
}

/// Face axis and whether the face points toward positive coordinates.
pub fn face_direction(vertex: [u32; 2]) -> (usize, bool) {
    let direction = vertex[0] >> 15 & 7;
    (direction as usize / 2, direction & 1 != 0)
}

struct EdgeIndices {
    vertices: [u32; EDGE_GRID],
    len: usize,
}

/// Index width of a [`TerrainGeometry`]. Opaque, shadow and water faces stay
/// under 65 536 vertices per section (a checkerboard has 49 152), so they mesh
/// straight to `u16`. Cutout cubes keep faces against each other and can
/// exceed it: they mesh to `u32` and narrow when the vertex count allows it.
/// Splitting rectangle edges adds at most the vertices of unmerged unit faces,
/// so the opaque section bound still holds after removing T-junctions.
pub trait TerrainIndex: Copy {
    fn new(index: u32) -> Self;
    fn into_indices(indices: Vec<Self>, vertices: usize) -> Indices;
}
impl TerrainIndex for u16 {
    #[inline]
    fn new(index: u32) -> Self {
        index as u16
    }
    fn into_indices(indices: Vec<Self>, vertices: usize) -> Indices {
        debug_assert!(vertices <= usize::from(u16::MAX) + 1);
        Indices::U16(indices)
    }
}
impl TerrainIndex for u32 {
    #[inline]
    fn new(index: u32) -> Self {
        index
    }
    fn into_indices(indices: Vec<Self>, vertices: usize) -> Indices {
        super::model_mesh::compact_indices(indices, vertices)
    }
}

impl<I: TerrainIndex> Default for TerrainGeometry<I> {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }
}

impl TerrainGeometry {
    pub fn position(vertex: [u32; 2]) -> Vec3 {
        UVec3::from_array(grid_position(vertex)).as_vec3()
    }
}

impl<I: TerrainIndex> TerrainGeometry<I> {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    fn push_quad(
        &mut self,
        corner: [i32; 3],
        du: [i32; 3],
        dv: [i32; 3],
        axis: usize,
        positive: bool,
        appearance: u16,
        word1: [u16; 4],
    ) {
        let add = |a: [i32; 3], b: [i32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let corners = [
            corner,
            add(corner, du),
            add(add(corner, du), dv),
            add(corner, dv),
        ];
        let first = self.vertices.len() as u32;
        let face = (axis as u32 * 2 + u32::from(positive)) << 15 | u32::from(appearance) << 18;
        self.vertices
            .extend(corners.into_iter().zip(word1).map(|(point, word1)| {
                let [x, y, z] = point.map(|value| value as u32);
                [x | y << 5 | z << 10 | face, u32::from(word1)]
            }));
        self.push_triangle([first, first + 1, first + 2], positive);
        self.push_triangle([first, first + 2, first + 3], positive);
    }

    /// Match long greedy edges to the short edges that meet them. Positions,
    /// textures and depth planes remain exact; only the triangulation changes.
    fn stitch_edges(&mut self, edges: &EdgeVertices) {
        let original_vertices = self.vertices.len();
        self.indices.clear();
        for first in (0..original_vertices).step_by(4) {
            let corners = std::array::from_fn::<_, 4, _>(|i| self.vertices[first + i]);
            let points = corners.map(grid_position);
            let (axis, positive) = face_direction(corners[0]);
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let splits = [
                edges.splits(points[0], points[1], u),
                edges.splits(points[2], points[1], v),
                edges.splits(points[2], points[3], u),
                edges.splits(points[0], points[3], v),
            ];
            let first = first as u32;
            if splits == [0; 4] {
                self.push_triangle([first, first + 1, first + 2], positive);
                self.push_triangle([first, first + 2, first + 3], positive);
                continue;
            }
            // Nonuniform lighting is never merged beyond one cell, so every
            // rectangle with an interior grid point has constant corner light.
            debug_assert!(corners.iter().all(|vertex| vertex[1] == corners[0][1]));
            let bottom = self.split_edge(first, first + 1, u, splits[0]);
            let right = self.split_edge(first + 2, first + 1, v, splits[1]);
            self.triangulate_chains(&bottom, &right, positive);
            let top = self.split_edge(first + 2, first + 3, u, splits[2]);
            let left = self.split_edge(first, first + 3, v, splits[3]);
            self.triangulate_chains(&top, &left, positive);
        }
    }

    fn split_edge(&mut self, first: u32, last: u32, axis: usize, mut splits: u32) -> EdgeIndices {
        let start = self.vertices[first as usize];
        let shift = axis * 5;
        let descending =
            grid_position(start)[axis] > grid_position(self.vertices[last as usize])[axis];
        let mut edge = EdgeIndices {
            vertices: [0; EDGE_GRID],
            len: 1,
        };
        edge.vertices[0] = first;
        while splits != 0 {
            let coordinate = if descending {
                31 - splits.leading_zeros()
            } else {
                splits.trailing_zeros()
            };
            splits &= !(1 << coordinate);
            edge.vertices[edge.len] = self.vertices.len() as u32;
            edge.len += 1;
            self.vertices
                .push([(start[0] & !(31 << shift)) | coordinate << shift, start[1]]);
        }
        edge.vertices[edge.len] = last;
        edge.len += 1;
        edge
    }

    /// Each half-rectangle has two boundary chains meeting at one corner.
    /// Advance along them alternately to include collinear boundary vertices
    /// without degenerate triangles or introducing another T-junction inside.
    fn triangulate_chains(&mut self, a: &EdgeIndices, b: &EdgeIndices, positive: bool) {
        let (mut i, mut j) = (0, 0);
        while i + 2 < a.len || j + 2 < b.len {
            if i + 2 < a.len {
                self.push_triangle([a.vertices[i], a.vertices[i + 1], b.vertices[j]], positive);
                i += 1;
            }
            if j + 2 < b.len {
                self.push_triangle([a.vertices[i], b.vertices[j + 1], b.vertices[j]], positive);
                j += 1;
            }
        }
        self.push_triangle(
            [a.vertices[i], a.vertices[a.len - 1], b.vertices[j]],
            positive,
        );
    }

    fn push_triangle(&mut self, mut vertices: [u32; 3], positive: bool) {
        if !positive {
            vertices.swap(1, 2);
        }
        self.indices.extend(vertices.map(I::new));
    }

    pub fn into_mesh(self) -> Mesh {
        let (min, max) = self
            .vertices
            .iter()
            .map(|&vertex| TerrainGeometry::position(vertex))
            .fold((Vec3::MAX, Vec3::MIN), |(min, max), position| {
                (min.min(position), max.max(position))
            });
        let indices = I::into_indices(self.indices, self.vertices.len());
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(ATTRIBUTE_VOXEL, self.vertices)
        .with_inserted_indices(indices);
        // Bevy derives bounds from `ATTRIBUTE_POSITION`, which packed meshes lack.
        mesh.final_aabb = min.cmple(max).all().then(|| Aabb3d::from_min_max(min, max));
        mesh
    }

    #[cfg(feature = "dev")]
    pub fn counts(&self) -> (usize, usize) {
        (self.vertices.len(), self.indices.len() / 3)
    }
}

/// Meshes the requested sections of a chunk; sections without blocks yield no geometry.
pub fn build_sections<C: RenderSource>(
    neighborhood: &ChunkNeighborhood<C>,
    sections: u32,
    shadows_enabled: bool,
) -> Vec<SectionMeshes> {
    SCRATCH.with_borrow_mut(|scratch| {
        let Scratch {
            padded,
            faces,
            water: water_faces,
            depths,
            model_depths,
            visited,
            flood_stack,
        } = scratch;
        (0..neighborhood.layout.section_count())
            .filter(|section| sections & (1 << section) != 0)
            .map(|index| {
                let (opaque, water) = neighborhood.section_contents(index);
                if !opaque && !water {
                    return SectionMeshes {
                        connectivity: super::visibility::ALL_CONNECTIONS,
                        index,
                        cutout: None,
                        terrain: None,
                        models: None,
                        models_cutout: None,
                        translucent: None,
                        shadow: None,
                    };
                }
                padded.fill_section(neighborhood, index);
                if padded.summary.enclosed && padded.summary.flags & cell::MODEL == 0 {
                    return SectionMeshes {
                        index,
                        connectivity: 0,
                        ..default()
                    };
                }
                let connectivity = neighborhood.cached_connectivity[index].unwrap_or_else(|| {
                    *visited = padded.summary.opaque;
                    super::visibility::connectivity(visited, flood_stack)
                });
                let casts_shadow = padded.summary.sky;
                let super::model_mesh::ModelSection {
                    opaque: models,
                    cutout: models_cutout,
                    mut translucent,
                } = if padded.summary.flags & cell::MODEL != 0 || padded.summary.fluids {
                    super::model_mesh::section(
                        padded,
                        neighborhood.origin + IVec3::Y * (index as i32 * SECTION_HEIGHT),
                        neighborhood,
                        index as i32 * SECTION_HEIGHT,
                        model_depths,
                    )
                } else {
                    default()
                };
                // Water only needs per-cell sort keys when it shares the
                // translucent mesh with ice, glass or other translucent models.
                let interleaved = translucent.is_some();
                let water = water
                    .then(|| {
                        ocean_mesh(
                            padded,
                            neighborhood,
                            index as i32 * SECTION_HEIGHT,
                            interleaved,
                            water_faces,
                            depths,
                        )
                    })
                    .flatten();
                if let Some(water) = water {
                    translucent
                        .get_or_insert_default()
                        .append_ocean(water, &neighborhood.source);
                }
                if padded.summary.flags & (cell::CUBE | cell::CUTOUT) != 0 {
                    padded.prepare_rows();
                }
                let mut terrain = (padded.summary.flags & cell::CUBE != 0)
                    .then(|| greedy_mesh::<u16>(padded, cell::CUBE, None, faces))
                    .filter(|geometry| !geometry.is_empty());
                let mut cutout = (padded.summary.flags & cell::CUTOUT != 0)
                    .then(|| greedy_mesh::<u32>(padded, cell::CUTOUT, None, faces))
                    .filter(|geometry| !geometry.is_empty());
                if terrain.is_some() || cutout.is_some() {
                    // Share endpoints across material boundaries as well as faces
                    // and orientations, including where leaves meet solid blocks.
                    let vertices = terrain
                        .iter()
                        .flat_map(|geometry| geometry.vertices.iter())
                        .chain(cutout.iter().flat_map(|geometry| geometry.vertices.iter()))
                        .copied();
                    let edges = EdgeVertices::new(vertices, padded);
                    if let Some(terrain) = &mut terrain {
                        terrain.stitch_edges(&edges);
                    }
                    if let Some(cutout) = &mut cutout {
                        cutout.stitch_edges(&edges);
                    }
                }
                SectionMeshes {
                    connectivity,
                    index,
                    models,
                    models_cutout,
                    translucent,
                    terrain,
                    cutout,
                    shadow: (shadows_enabled
                        && padded.summary.flags & cell::CUBE != 0
                        && casts_shadow)
                        .then(|| {
                            greedy_mesh::<u16>(
                                padded,
                                cell::CUBE,
                                Some(neighborhood.source.shadow_directions()),
                                faces,
                            )
                        })
                        .filter(|geometry| !geometry.is_empty()),
                }
            })
            .collect()
    })
}

/// Cull whole padded rows before reading materials or calculating corner light.
/// Shadows reuse cube occupancy, but ignore material/light and backlit faces.
fn greedy_mesh<I: TerrainIndex>(
    padded: &Padded,
    class: u8,
    shadow: Option<[bool; 3]>,
    mask: &mut Vec<Option<LitFace>>,
) -> TerrainGeometry<I> {
    let dimensions = padded.dimensions;
    let mut geometry = TerrainGeometry::default();
    mask.resize((CHUNK_SIZE * CHUNK_SIZE) as usize, None);
    let class_shift = if class == cell::CUTOUT { ROW_BITS } else { 0 };
    // Cutouts retain both faces between neighbors; only occluders hide terrain.
    // The shadow hull instead follows cube occupancy, as in the depth-only pass.
    let blocker_shift = if shadow.is_some() { 0 } else { 2 * ROW_BITS };
    for axis in 0..3 {
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let (width, height) = (dimensions[u] as usize, dimensions[v] as usize);
        let width_mask = (1u32 << width) - 1;
        for layer in 0..dimensions[axis] {
            for positive in [true, false] {
                // Two-sided shadow rasterization can omit backlit hull faces:
                // an opposing face of the closed surface supplies the same depth.
                if shadow.is_some_and(|excluded| !positive && excluded[axis]) {
                    continue;
                }
                let mut rows = [0u32; CHUNK_SIZE as usize];
                for (j, row) in rows[..height].iter_mut().enumerate() {
                    let index = axis * AXIS_ROWS + layer as usize + 1 + ROW_BITS * (j + 1);
                    let neighbor = if positive { index + 1 } else { index - 1 };
                    *row = (((padded.rows[index] >> class_shift)
                        & !(padded.rows[neighbor] >> blocker_shift))
                        >> 1) as u32
                        & width_mask;
                    if shadow.is_none() {
                        let mut pending = *row;
                        while pending != 0 {
                            let i = pending.trailing_zeros() as usize;
                            pending &= pending - 1;
                            let mut outside = [0; 3];
                            outside[axis] = layer + if positive { 1 } else { -1 };
                            outside[u] = i as i32;
                            outside[v] = j as i32;
                            let mut cell = outside;
                            cell[axis] = layer;
                            let appearance = padded.get(IVec3::from_array(cell)).info.appearance;
                            let face =
                                Face(appearance, positive, face_light(padded, outside, axis));
                            let index = i + j * width;
                            mask[index] = Some(LitFace::new(face, index, false));
                        }
                    }
                }
                let emit = |i, j, w, h, face: LitFace| {
                    let (corner, du, dv) = quad(axis, layer - i32::from(!positive), i, j, w, h);
                    let Face(appearance, positive, light) = face.face;
                    geometry.push_quad(corner, du, dv, axis, positive, appearance, light);
                };
                if shadow.is_some() {
                    consume_binary_mask(
                        &mut rows[..height],
                        width,
                        |_| LitFace {
                            face: Face(0, positive, [0; 4]),
                            cell: None,
                        },
                        emit,
                    );
                } else {
                    consume_binary_mask(
                        &mut rows[..height],
                        width,
                        |index| mask[index].unwrap(),
                        emit,
                    );
                }
            }
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

/// Water columns deeper than this read as fully deep in the shader.
const WATER_DEPTH_CAP: usize = 8;
/// Depth is packed in the bits above the 12 bits of corner light, as 0..=15.
const WATER_DEPTH_SHIFT: u16 = 12;
const WATER_DEPTH_MAX: u16 = 15;
/// Marks corners at the lowered surface level, 2/16 below the block top
/// (`water_material.wesl` applies the drop).
const WATER_LOWERED: u32 = 1 << 16;

/// Water cells stacked downward from `top` (inclusive), capped at [`WATER_DEPTH_CAP`].
pub fn water_depth<C: RenderSource>(neighborhood: &ChunkNeighborhood<C>, top: IVec3) -> u8 {
    let family = neighborhood.sample(top).fluid_id();
    if family == hollow_core::FluidId::NONE {
        return 0;
    }
    (0..WATER_DEPTH_CAP as i32)
        .take_while(|&below| neighborhood.sample(top - IVec3::Y * below).fluid_id() == family)
        .count() as u8
}

/// Corner depths of a surface face, averaged over the four columns around each
/// corner so shores fade smoothly into shallow water.
pub fn surface_depth(depths: &[u8], stride: i32, x: i32, z: i32) -> [u16; 4] {
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

fn ocean_mesh<C: RenderSource>(
    padded: &Padded,
    neighborhood: &ChunkNeighborhood<C>,
    base: i32,
    interleaved: bool,
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
            let mut rows = [0u32; CHUNK_SIZE as usize];
            for j in 0..height {
                for i in 0..width {
                    let mut cell = [0; 3];
                    cell[axis] = layer;
                    cell[u] = i as i32;
                    cell[v] = j as i32;
                    let a = padded.get(IVec3::from_array(cell));
                    cell[axis] += 1;
                    let b = padded.get(IVec3::from_array(cell));
                    // Only exposed water interfaces. Transparent solid cubes
                    // also enclose water: their internal contact must not add
                    // a reflective free surface behind ice or glass.
                    let (positive, water_cell, outside) =
                        match (a.ocean_surface(), b.ocean_surface()) {
                            (true, false) if layer >= 0 && !b.hides_fluid() => {
                                let mut water_cell = cell;
                                water_cell[axis] -= 1;
                                (true, water_cell, cell)
                            }
                            (false, true) if layer + 1 < dimensions[axis] && !a.hides_fluid() => {
                                let mut outside = cell;
                                outside[axis] -= 1;
                                (false, cell, outside)
                            }
                            _ => continue,
                        };
                    let water_cell = IVec3::from_array(water_cell);
                    // Uneven surfaces use the shared explicit fluid builder. Broad
                    // flat oceans retain greedy merging and compact water vertices.
                    if super::fluids::explicit_surface(padded, &neighborhood.source, water_cell) {
                        continue;
                    }
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
                    // Lower only a free surface. Water beneath a solid ceiling
                    // fills up to it, including the upper rim of any exposed side.
                    let above = padded.get(water_cell + IVec3::Y);
                    let lowered =
                        surface || (axis != 1 && !above.ocean_surface() && !above.hides_fluid());
                    let face = Face(padded.get(water_cell).fluid_id().0, positive, values);
                    // Voxel-sized water faces can interleave with ice and glass:
                    // even uniform neighbors keep their own depth keys.
                    mask[i + j * width] =
                        Some((LitFace::new(face, i + j * width, interleaved), lowered));
                    rows[j] |= 1 << i;
                }
            }
            consume_binary_mask(
                &mut rows[..height],
                width,
                |index| mask[index].unwrap(),
                |i, j, w, h, (face, lowered)| {
                    let (corner, du, dv) = quad(axis, layer, i, j, w, h);
                    let Face(appearance, positive, values) = face.face;
                    geometry.push_quad(corner, du, dv, axis, positive, appearance, values);
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
                },
            );
        }
    }
    (!geometry.is_empty()).then_some(geometry)
}

/// Row bits skip empty cells and clear merged rectangles without revisiting cells.
fn consume_binary_mask<T: Copy + Eq>(
    rows: &mut [u32],
    width: usize,
    value_at: impl Fn(usize) -> T,
    mut emit: impl FnMut(usize, usize, usize, usize, T),
) {
    for y in 0..rows.len() {
        while rows[y] != 0 {
            let x = rows[y].trailing_zeros() as usize;
            let value = value_at(x + y * width);
            let run = (rows[y] >> x).trailing_ones() as usize;
            let mut quad_width = 1;
            while quad_width < run && value_at(x + quad_width + y * width) == value {
                quad_width += 1;
            }
            let span = ((1u32 << quad_width) - 1) << x;
            let mut quad_height = 1;
            while y + quad_height < rows.len()
                && rows[y + quad_height] & span == span
                && (0..quad_width).all(|dx| value_at(x + dx + (y + quad_height) * width) == value)
            {
                quad_height += 1;
            }
            for row in &mut rows[y..y + quad_height] {
                *row &= !span;
            }
            emit(x, y, quad_width, quad_height, value);
        }
    }
}
