//! Minecraft's unfolded cuboid UV layout, shared by skins and armor.
use bevy::{asset::RenderAssetUsages, mesh::Indices, prelude::*};

pub(crate) const PIXEL: f32 = 1.8 / 32.0;

pub(crate) fn cuboid(size: Vec3, center: Vec3, uv: Vec2, texture: Vec2, inflate: f32) -> Mesh {
    let half = size * 0.5 + Vec3::splat(inflate);
    let [w, h, d] = size.to_array();
    let mut mesh = Cuboid { half_size: half }.mesh().build();
    mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
    if let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for p in positions {
            *p = ((Vec3::from_array(*p) + center) * PIXEL).to_array();
        }
    }
    // Bevy orders faces +Z, -Z, +X, -X, +Y, -Y. Adapt its canonical
    // coordinates to Minecraft's unfolded skin layout without rebuilding geometry.
    let rectangles = [
        [d, d, w, h],
        [d + w + d, d, w, h],
        [d + w, d, d, h],
        [0., d, d, h],
        [d, 0., w, d],
        [d + w, 0., w, d],
    ];
    if let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
    {
        for (face, vertices) in uvs.chunks_exact_mut(4).enumerate() {
            let [u, v, width, height] = rectangles[face];
            for vertex in vertices {
                let [x, y] = *vertex;
                let local = match face {
                    0 => Vec2::new(x, 1. - y),
                    2 => Vec2::new(1. - y, 1. - x),
                    3 => Vec2::new(1. - y, x),
                    5 => Vec2::new(1. - x, 1. - y),
                    _ => Vec2::new(x, y),
                };
                *vertex = ((Vec2::new(u, v) + local * Vec2::new(width, height) + uv) / texture)
                    .to_array();
            }
        }
    }
    if let Some(Indices::U32(indices)) = mesh.remove_indices() {
        mesh.insert_indices(Indices::U16(
            indices.into_iter().map(|i| i as u16).collect(),
        ));
    }
    mesh.final_aabb = Some(Aabb3d::from_min_max(
        (center - half) * PIXEL,
        (center + half) * PIXEL,
    ));
    mesh
}

/// Model-space size of each body part: head, body, right/left arm, right/left leg.
fn part_size(part: usize) -> Vec3 {
    match part {
        0 => Vec3::splat(8.),
        1 => Vec3::new(8., 12., 4.),
        _ => Vec3::new(4., 12., 4.),
    }
}

fn part_center(part: usize) -> Vec3 {
    Vec3::Y * if part == 0 { 4. } else { -6. }
}

/// Armor uses the classic proportions and the 64x32 equipment texture layout.
pub(super) fn armor_parts(meshes: &mut Assets<Mesh>, inflate: f32) -> [Handle<Mesh>; 6] {
    const UV: [Vec2; 6] = [
        Vec2::ZERO,
        Vec2::new(16., 16.),
        Vec2::new(40., 16.),
        Vec2::new(40., 16.),
        Vec2::new(0., 16.),
        Vec2::new(0., 16.),
    ];
    std::array::from_fn(|part| {
        meshes.add(cuboid(
            part_size(part),
            part_center(part),
            UV[part],
            Vec2::new(64., 32.),
            inflate,
        ))
    })
}

pub(super) fn skin_parts(meshes: &mut Assets<Mesh>) -> [[Handle<Mesh>; 6]; 2] {
    let base = [
        Vec2::new(0., 0.),
        Vec2::new(16., 16.),
        Vec2::new(40., 16.),
        Vec2::new(32., 48.),
        Vec2::new(0., 16.),
        Vec2::new(16., 48.),
    ];
    let outer = [
        Vec2::new(32., 0.),
        Vec2::new(16., 32.),
        Vec2::new(40., 32.),
        Vec2::new(48., 48.),
        Vec2::new(0., 32.),
        Vec2::new(0., 48.),
    ];
    std::array::from_fn(|layer| {
        std::array::from_fn(|part| {
            meshes.add(cuboid(
                part_size(part),
                part_center(part),
                if layer == 0 { base[part] } else { outer[part] },
                Vec2::splat(64.),
                if layer == 0 {
                    0.
                } else if part == 0 {
                    0.5
                } else {
                    0.25
                },
            ))
        })
    })
}
