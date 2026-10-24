use super::{ChunkVoxels, VoxelKind, data::SECTION_HEIGHT, material::TexturePack};
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};
use std::sync::Arc;

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
    light: 0xf0,
    occludes: false,
};
const BELOW_WORLD: Sample = Sample {
    kind: VoxelKind::Air,
    light: 0,
    occludes: true,
};

#[derive(Clone)]
struct Column {
    cells: Arc<[VoxelKind]>,
    light: Arc<[u8]>,
}

/// A chunk and its eight neighbors, captured in O(1) for a mesh task.
///
/// Missing neighbors read as air (their border faces are drawn) with the chunk's own
/// edge light, so world borders are neither dark nor artificially lit.
#[derive(Clone)]
pub(crate) struct ChunkNeighborhood {
    size: i32,
    height: i32,
    columns: [Option<Column>; 9],
}

impl ChunkNeighborhood {
    pub(crate) fn new<'a>(
        center: &ChunkVoxels,
        neighbor: impl Fn(IVec2) -> Option<&'a ChunkVoxels>,
    ) -> Self {
        let column = |voxels: &ChunkVoxels| Column {
            cells: voxels.cells().clone(),
            light: voxels.light.clone(),
        };
        let columns = std::array::from_fn(|slot| {
            let offset = IVec2::new(slot as i32 % 3 - 1, slot as i32 / 3 - 1);
            if offset == IVec2::ZERO {
                return Some(column(center));
            }
            neighbor(offset)
                .filter(|voxels| voxels.size == center.size && voxels.height == center.height)
                .map(column)
        });
        Self {
            size: center.size,
            height: center.height,
            columns,
        }
    }

    /// Bits of the 3x3 slots without a loaded neighbor.
    pub(crate) fn missing_neighbors(&self) -> u16 {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.is_none())
            .fold(0, |mask, (slot, _)| mask | (1 << slot))
    }

    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        (x + self.size * (z + self.size * y)) as usize
    }

    fn sample(&self, local: IVec3) -> Sample {
        if local.y >= self.height {
            return ABOVE_WORLD;
        }
        if local.y < 0 {
            return BELOW_WORLD;
        }
        let slot = |value: i32| {
            if value < 0 {
                0
            } else if value >= self.size {
                2
            } else {
                1
            }
        };
        if let Some(column) = &self.columns[slot(local.x) + 3 * slot(local.z)] {
            let index = self.index(
                local.x.rem_euclid(self.size),
                local.y,
                local.z.rem_euclid(self.size),
            );
            let kind = column.cells[index];
            return Sample {
                kind,
                light: column.light[index],
                occludes: kind.is_opaque(),
            };
        }
        let center = self.columns[4].as_ref().expect("center column");
        let index = self.index(
            local.x.clamp(0, self.size - 1),
            local.y,
            local.z.clamp(0, self.size - 1),
        );
        Sample {
            kind: VoxelKind::Air,
            light: center.light[index],
            occludes: center.cells[index].is_opaque(),
        }
    }

    fn section_contents(&self, section: usize) -> (bool, bool) {
        let center = self.columns[4].as_ref().expect("center column");
        let area = (self.size * self.size) as usize;
        let start = section * SECTION_HEIGHT as usize * area;
        let end = (start + SECTION_HEIGHT as usize * area).min(center.cells.len());
        center.cells[start..end]
            .iter()
            .fold((false, false), |(opaque, water), kind| {
                (opaque || kind.is_opaque(), water || *kind == VoxelKind::Water)
            })
    }
}

/// One section plus a one-cell border, copied once so meshing indexes a flat array.
struct Padded {
    dimensions: [i32; 3],
    samples: Vec<Sample>,
}

impl Padded {
    fn section(neighborhood: &ChunkNeighborhood, section: usize) -> Self {
        let base = section as i32 * SECTION_HEIGHT;
        let height = SECTION_HEIGHT.min(neighborhood.height - base);
        let size = neighborhood.size;
        let mut samples = Vec::with_capacity(((size + 2) * (size + 2) * (height + 2)) as usize);
        for y in -1..=height {
            for z in -1..=size {
                for x in -1..=size {
                    samples.push(neighborhood.sample(IVec3::new(x, base + y, z)));
                }
            }
        }
        Self {
            dimensions: [size, height, size],
            samples,
        }
    }

    fn single(kind: VoxelKind) -> Self {
        let mut samples = vec![ABOVE_WORLD; 27];
        samples[13] = Sample {
            kind,
            light: 0xf0,
            occludes: kind.is_opaque(),
        };
        Self {
            dimensions: [1, 1, 1],
            samples,
        }
    }

    fn get(&self, local: IVec3) -> Sample {
        let width = self.dimensions[0] + 2;
        let depth = self.dimensions[2] + 2;
        self.samples
            [((local.x + 1) + width * ((local.z + 1) + depth * (local.y + 1))) as usize]
    }
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
    pub(crate) terrain: Option<VoxelGeometry>,
    pub(crate) water: Option<VoxelGeometry>,
    /// Sunlight reaches the section or its border; otherwise its faces are buried
    /// beneath other geometry and can be left out of the shadow maps.
    pub(crate) casts_shadow: bool,
}

#[derive(Default)]
pub(crate) struct VoxelGeometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    light: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl VoxelGeometry {
    pub(crate) fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub(crate) fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, self.light)
        .with_inserted_indices(Indices::U32(self.indices))
    }

    #[cfg(feature = "dev")]
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.positions.len(), self.indices.len() / 3)
    }
}

/// Meshes the requested sections of a chunk; sections without blocks yield no geometry.
pub(crate) fn build_sections(
    neighborhood: &ChunkNeighborhood,
    sections: u32,
    texture_pack: &TexturePack,
) -> Vec<SectionMeshes> {
    let count = (neighborhood.height + SECTION_HEIGHT - 1) / SECTION_HEIGHT;
    (0..count as usize)
        .filter(|section| sections & (1 << section) != 0)
        .map(|index| {
            let (opaque, water) = neighborhood.section_contents(index);
            if !opaque && !water {
                return SectionMeshes {
                    index,
                    terrain: None,
                    water: None,
                    casts_shadow: false,
                };
            }
            let padded = Padded::section(neighborhood, index);
            SectionMeshes {
                index,
                casts_shadow: padded.samples.iter().any(|sample| sample.light >> 4 != 0),
                terrain: opaque
                    .then(|| greedy_mesh(&padded, texture_pack, true))
                    .filter(|geometry| !geometry.is_empty()),
                water: water.then(|| water_mesh(&padded, texture_pack)).flatten(),
            }
        })
        .collect()
}

pub(crate) fn build_block_item_mesh(kind: VoxelKind, texture_pack: &TexturePack) -> Mesh {
    greedy_mesh(&Padded::single(kind), texture_pack, false)
        .into_mesh()
        .translated_by(Vec3::splat(-0.5))
}

fn greedy_mesh(padded: &Padded, texture_pack: &TexturePack, lit: bool) -> VoxelGeometry {
    let dimensions = padded.dimensions;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut light = Vec::new();
    let mut indices = Vec::new();
    let largest = dimensions[0].max(dimensions[1]).max(dimensions[2]);
    let mut mask = vec![None; (largest * largest) as usize];
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
                    let mut corner = [0; 3];
                    corner[axis] = layer + 1;
                    corner[u] = i as i32;
                    corner[v] = j as i32;
                    let mut du = [0; 3];
                    let mut dv = [0; 3];
                    du[u] = quad_width as i32;
                    dv[v] = quad_height as i32;
                    push_quad(
                        &mut positions,
                        &mut normals,
                        &mut colors,
                        &mut uvs,
                        &mut light,
                        &mut indices,
                        corner,
                        du,
                        dv,
                        axis,
                        face.face,
                        true,
                        texture_pack,
                    );
                },
            );
        }
    }

    finish_mesh(positions, normals, colors, uvs, light, indices)
}

fn water_mesh(padded: &Padded, texture_pack: &TexturePack) -> Option<VoxelGeometry> {
    let dimensions = padded.dimensions;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut light = Vec::new();
    let mut indices = Vec::new();
    for axis in 0..3 {
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let (width, height) = (dimensions[u] as usize, dimensions[v] as usize);
        let mut mask = vec![None; width * height];
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
                    // Only real air/fluid interfaces. No water plane inside a solid
                    // ceiling, no interior faces, and no faces owned by a neighbor.
                    mask[i + j * width] = match (a, b) {
                        (VoxelKind::Water, VoxelKind::Air) if layer >= 0 => Some(LitFace::new(
                            Face(VoxelKind::Water, true, face_light(padded, cell, axis)),
                            i + j * width,
                        )),
                        (VoxelKind::Air, VoxelKind::Water) if layer + 1 < dimensions[axis] => {
                            cell[axis] -= 1;
                            Some(LitFace::new(
                                Face(VoxelKind::Water, false, face_light(padded, cell, axis)),
                                i + j * width,
                            ))
                        }
                        _ => None,
                    };
                }
            }
            consume_mask(&mut mask, width, height, |i, j, w, h, face| {
                let mut corner = [0; 3];
                corner[axis] = layer + 1;
                corner[u] = i as i32;
                corner[v] = j as i32;
                let mut du = [0; 3];
                let mut dv = [0; 3];
                du[u] = w as i32;
                dv[v] = h as i32;
                push_quad(
                    &mut positions,
                    &mut normals,
                    &mut colors,
                    &mut uvs,
                    &mut light,
                    &mut indices,
                    corner,
                    du,
                    dv,
                    axis,
                    face.face,
                    false,
                    texture_pack,
                );
            });
        }
    }
    (!indices.is_empty()).then(|| finish_mesh(positions, normals, colors, uvs, light, indices))
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

fn finish_mesh(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    light: Vec<[f32; 2]>,
    indices: Vec<u32>,
) -> VoxelGeometry {
    VoxelGeometry {
        positions,
        normals,
        colors,
        uvs,
        light,
        indices,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    uvs: &mut Vec<[f32; 2]>,
    light: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    corner: [i32; 3],
    du: [i32; 3],
    dv: [i32; 3],
    axis: usize,
    face: Face,
    textured: bool,
    texture_pack: &TexturePack,
) {
    let add = |a: [i32; 3], b: [i32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    let vertices = [
        corner,
        add(corner, du),
        add(add(corner, du), dv),
        add(corner, dv),
    ];
    positions.extend(vertices.map(|point| point.map(|value| value as f32)));
    let mut normal = [0.0; 3];
    normal[axis] = if face.1 { 1.0 } else { -1.0 };
    normals.extend([normal; 4]);
    if textured {
        let tint = texture_pack.texture_tint(face.0, axis, face.1);
        let layer = texture_pack.texture_layer(face.0, axis, face.1) as f32;
        colors.extend([[tint[0], tint[1], tint[2], layer]; 4]);
    } else {
        colors.extend([face.0.color(); 4]);
    }
    uvs.extend(quad_uvs(axis, du, dv));
    light.extend(
        face.2
            .map(|value| [f32::from(value & 63) / 60.0, f32::from(value >> 6) / 60.0]),
    );
    let first = positions.len() as u32 - 4;
    let order = if face.1 {
        [0, 1, 2, 0, 2, 3]
    } else {
        [0, 2, 1, 0, 3, 2]
    };
    indices.extend(order.map(|index| first + index));
}

fn quad_uvs(axis: usize, du: [i32; 3], dv: [i32; 3]) -> [[f32; 2]; 4] {
    match axis {
        0 => {
            let (width, height) = (dv[2] as f32, du[1] as f32);
            [[0.0, height], [0.0, 0.0], [width, 0.0], [width, height]]
        }
        1 => {
            let (width, height) = (du[2] as f32, dv[0] as f32);
            [[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]]
        }
        _ => {
            let (width, height) = (du[0] as f32, dv[1] as f32);
            [[0.0, height], [width, height], [width, 0.0], [0.0, 0.0]]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> TexturePack {
        TexturePack::load(super::super::TexturePackId::default())
    }

    fn mesh(center: &ChunkVoxels, neighbor: Option<(IVec2, &ChunkVoxels)>) -> SectionMeshes {
        let neighborhood = ChunkNeighborhood::new(center, |offset| {
            neighbor.and_then(|(position, voxels)| (position == offset).then_some(voxels))
        });
        build_sections(&neighborhood, 1, &pack()).remove(0)
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

    #[test]
    fn neighbors_never_emit_neighbor_owned_faces() {
        let stone = ChunkVoxels::generated(1, 1, vec![VoxelKind::Stone]);
        let empty = ChunkVoxels::generated(1, 1, vec![VoxelKind::Air]);
        for direction in [IVec2::NEG_X, IVec2::X, IVec2::NEG_Y, IVec2::Y] {
            assert!(mesh(&empty, Some((direction, &stone))).terrain.is_none());
            let solid = mesh(&stone, Some((direction, &stone))).terrain.unwrap();
            assert_eq!(solid.indices.len(), 5 * 6);
        }
        assert_eq!(mesh(&stone, None).terrain.unwrap().indices.len(), 6 * 6);
    }

    #[test]
    fn neighbor_light_reaches_border_face_corners() {
        let mut center = ChunkVoxels::generated(1, 1, vec![VoxelKind::Stone]);
        center = center.with_lighting(vec![0].into(), vec![0].into());
        let lit = ChunkVoxels::generated(1, 1, vec![VoxelKind::Air])
            .with_lighting(vec![0].into(), vec![0x37].into());
        let neighborhood = ChunkNeighborhood::new(&center, |offset| {
            (offset == IVec2::NEG_X).then_some(&lit)
        });
        let sample = neighborhood.sample(IVec3::new(-1, 0, 0));
        assert_eq!(sample.light, 0x37);
        assert!(!sample.occludes);
        assert_eq!(neighborhood.sample(IVec3::Y).light, 0xf0);
        assert!(neighborhood.sample(IVec3::NEG_Y).occludes);
        // A missing neighbor repeats the chunk's own edge.
        assert!(neighborhood.sample(IVec3::new(1, 0, 0)).occludes);
    }

    #[test]
    fn water_has_side_faces_and_no_faces_inside_solid_ceilings() {
        let mut water = ChunkVoxels::generated(1, 2, vec![VoxelKind::Water, VoxelKind::Air]);
        let geometry = mesh(&water, None).water.unwrap();
        assert_eq!(geometry.indices.len(), 6 * 6);
        assert!(geometry.normals.contains(&[1.0, 0.0, 0.0]));
        assert!(geometry.normals.contains(&[0.0, 1.0, 0.0]));
        water.set(IVec3::Y, VoxelKind::Stone);
        let geometry = mesh(&water, None).water.unwrap();
        assert_eq!(geometry.indices.len(), 5 * 6);
        assert!(!geometry.normals.contains(&[0.0, 1.0, 0.0]));
        water.set(IVec3::Y, VoxelKind::Water);
        let geometry = mesh(&water, None).water.unwrap();
        assert_eq!(geometry.indices.len(), 6 * 6);
    }

    #[test]
    fn sections_are_meshed_independently_and_empty_ones_are_skipped() {
        let mut cells = vec![VoxelKind::Air; 48];
        cells[40] = VoxelKind::Stone;
        let chunk = ChunkVoxels::generated(1, 48, cells);
        let neighborhood = ChunkNeighborhood::new(&chunk, |_| None);
        let sections = build_sections(&neighborhood, 0b111, &pack());
        assert_eq!(sections.len(), 3);
        assert!(sections[0].terrain.is_none() && sections[1].terrain.is_none());
        assert_eq!(sections[2].terrain.as_ref().unwrap().indices.len(), 6 * 6);
        // Positions are local to the section.
        assert!(
            sections[2].terrain.as_ref().unwrap().positions.iter().all(|p| p[1] <= 9.0)
        );
    }
}
