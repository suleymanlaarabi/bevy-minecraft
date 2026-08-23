use super::{ChunkVoxels, VoxelKind};
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Face(VoxelKind, bool);
pub(crate) fn build_chunk_mesh(voxels: &ChunkVoxels) -> Mesh {
    greedy_mesh(voxels.size, voxels.height, |position| {
        voxels.sample(position)
    })
}

fn greedy_mesh(size: i32, height: i32, sample: impl Fn(IVec3) -> VoxelKind) -> Mesh {
    let dimensions = [size, height, size];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
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
                    mask[i + j * width] = match (a.is_solid(), b.is_solid()) {
                        (true, false) => Some(Face(a, true)),
                        (false, true) => Some(Face(b, false)),
                        _ => None,
                    };
                }
            }

            let mut j = 0;
            while j < height {
                let mut i = 0;
                while i < width {
                    let Some(face) = mask[i + j * width] else {
                        i += 1;
                        continue;
                    };
                    let mut quad_width = 1;
                    while i + quad_width < width && mask[i + quad_width + j * width] == Some(face) {
                        quad_width += 1;
                    }
                    let mut quad_height = 1;
                    'grow: while j + quad_height < height {
                        for x in 0..quad_width {
                            if mask[i + x + (j + quad_height) * width] != Some(face) {
                                break 'grow;
                            }
                        }
                        quad_height += 1;
                    }

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
                        &mut indices,
                        corner,
                        du,
                        dv,
                        axis,
                        face,
                    );
                    for y in 0..quad_height {
                        for x in 0..quad_width {
                            mask[i + x + (j + y) * width] = None;
                        }
                    }
                    i += quad_width;
                }
                j += 1;
            }
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

#[allow(clippy::too_many_arguments)]
fn push_quad(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    corner: [i32; 3],
    du: [i32; 3],
    dv: [i32; 3],
    axis: usize,
    face: Face,
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
    colors.extend([face.0.color(); 4]);
    let first = positions.len() as u32 - 4;
    let order = if face.1 {
        [0, 1, 2, 0, 2, 3]
    } else {
        [0, 2, 1, 0, 3, 2]
    };
    indices.extend(order.map(|index| first + index));
}
