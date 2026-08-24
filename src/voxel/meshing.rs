use super::{ChunkVoxels, VoxelKind, material::TexturePack};
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Face(VoxelKind, bool);
pub(crate) struct ChunkMeshes(pub(crate) VoxelGeometry, pub(crate) Option<VoxelGeometry>);

#[derive(Default)]
pub(crate) struct VoxelGeometry {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl VoxelGeometry {
    pub(crate) fn append(&mut self, geometry: &Self, offset: Vec3) {
        let first = self.positions.len() as u32;
        self.positions
            .extend(geometry.positions.iter().map(|position| {
                [
                    position[0] + offset.x,
                    position[1] + offset.y,
                    position[2] + offset.z,
                ]
            }));
        self.normals.extend_from_slice(&geometry.normals);
        self.colors.extend_from_slice(&geometry.colors);
        self.uvs.extend_from_slice(&geometry.uvs);
        self.indices
            .extend(geometry.indices.iter().map(|index| first + index));
    }

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
        .with_inserted_indices(Indices::U32(self.indices))
    }

    #[cfg(feature = "dev")]
    fn counts(&self) -> (usize, usize) {
        (self.positions.len(), self.indices.len() / 3)
    }
}

#[cfg(feature = "dev")]
impl ChunkMeshes {
    pub(crate) fn geometry_counts(&self) -> (usize, usize) {
        let terrain = self.0.counts();
        let water = self.1.as_ref().map_or((0, 0), VoxelGeometry::counts);
        (terrain.0 + water.0, terrain.1 + water.1)
    }
}

pub(crate) fn build_chunk_mesh(voxels: &ChunkVoxels, texture_pack: &TexturePack) -> ChunkMeshes {
    ChunkMeshes(
        greedy_mesh(
            voxels.size,
            voxels.height,
            |position| voxels.sample(position),
            texture_pack,
        ),
        water_mesh(voxels, texture_pack),
    )
}

pub(crate) fn build_block_item_mesh(kind: VoxelKind, texture_pack: &TexturePack) -> Mesh {
    greedy_mesh(
        1,
        1,
        |position| {
            if position == IVec3::ZERO {
                kind
            } else {
                VoxelKind::Air
            }
        },
        texture_pack,
    )
    .into_mesh()
    .translated_by(Vec3::splat(-0.5))
}

fn greedy_mesh(
    size: i32,
    height: i32,
    sample: impl Fn(IVec3) -> VoxelKind,
    texture_pack: &TexturePack,
) -> VoxelGeometry {
    let dimensions = [size, height, size];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let mut mask = vec![None; (size * height).max(size * size) as usize];
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
                    let a = sample(IVec3::from_array(cell));
                    cell[axis] += 1;
                    let b = sample(IVec3::from_array(cell));
                    mask[i + j * width] = match (a.is_opaque(), b.is_opaque()) {
                        (true, false) => Some(Face(a, true)),
                        (false, true) => Some(Face(b, false)),
                        _ => None,
                    };
                }
            }

            consume_mask(
                &mut mask,
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
                        &mut indices,
                        corner,
                        du,
                        dv,
                        axis,
                        face,
                        true,
                        texture_pack,
                    );
                },
            );
        }
    }

    finish_mesh(positions, normals, colors, uvs, indices)
}

fn water_mesh(voxels: &ChunkVoxels, texture_pack: &TexturePack) -> Option<VoxelGeometry> {
    let size = voxels.size as usize;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let mut mask = vec![None; size * size];
    for y in 0..voxels.height {
        for z in 0..size {
            for x in 0..size {
                let position = IVec3::new(x as i32, y, z as i32);
                mask[x + z * size] = (voxels.sample(position) == VoxelKind::Water
                    && voxels.sample(position + IVec3::Y) != VoxelKind::Water)
                    .then_some(());
            }
        }
        consume_mask(&mut mask, size, size, |x, z, width, depth, ()| {
            push_quad(
                &mut positions,
                &mut normals,
                &mut colors,
                &mut uvs,
                &mut indices,
                [x as i32, y + 1, z as i32],
                [0, 0, depth as i32],
                [width as i32, 0, 0],
                1,
                Face(VoxelKind::Water, true),
                false,
                texture_pack,
            );
        });
    }
    (!indices.is_empty()).then(|| finish_mesh(positions, normals, colors, uvs, indices))
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
    indices: Vec<u32>,
) -> VoxelGeometry {
    VoxelGeometry {
        positions,
        normals,
        colors,
        uvs,
        indices,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    uvs: &mut Vec<[f32; 2]>,
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

    #[test]
    fn append_offsets_vertices_and_rebases_indices() {
        let geometry = VoxelGeometry {
            positions: vec![[0.0, 1.0, 2.0], [1.0, 1.0, 2.0], [1.0, 2.0, 2.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            colors: vec![[1.0; 4]; 3],
            uvs: vec![[0.0; 2]; 3],
            indices: vec![0, 1, 2],
        };
        let mut combined = VoxelGeometry::default();

        combined.append(&geometry, Vec3::new(4.0, 0.0, -2.0));
        combined.append(&geometry, Vec3::new(8.0, 0.0, -2.0));

        assert_eq!(combined.positions[0], [4.0, 1.0, 0.0]);
        assert_eq!(combined.positions[3], [8.0, 1.0, 0.0]);
        assert_eq!(combined.indices, [0, 1, 2, 3, 4, 5]);
    }
}
