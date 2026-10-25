//! Minecraft's unfolded cuboid UV layout, shared by skins and armor.
use bevy::{
    asset::RenderAssetUsages, mesh::Indices, prelude::*, render::render_resource::PrimitiveTopology,
};

pub(super) const PIXEL: f32 = 1.8 / 32.0;

pub(super) fn cuboid(size: Vec3, center: Vec3, uv: Vec2, texture: Vec2, inflate: f32) -> Mesh {
    let half = size * 0.5 + Vec3::splat(inflate);
    let [x, y, z] = half.to_array();
    let [w, h, d] = size.to_array();
    let mut positions = Vec::with_capacity(24);
    let mut normals = Vec::with_capacity(24);
    let mut uvs = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    let faces = [
        (
            [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]],
            Vec3::Z,
            [d, d, w, h],
        ),
        (
            [[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]],
            Vec3::NEG_Z,
            [d + w + d, d, w, h],
        ),
        (
            [[-x, -y, -z], [-x, -y, z], [-x, y, z], [-x, y, -z]],
            Vec3::NEG_X,
            [0., d, d, h],
        ),
        (
            [[x, -y, z], [x, -y, -z], [x, y, -z], [x, y, z]],
            Vec3::X,
            [d + w, d, d, h],
        ),
        (
            [[-x, y, z], [x, y, z], [x, y, -z], [-x, y, -z]],
            Vec3::Y,
            [d, 0., w, d],
        ),
        (
            [[-x, -y, -z], [x, -y, -z], [x, -y, z], [-x, -y, z]],
            Vec3::NEG_Y,
            [d + w, 0., w, d],
        ),
    ];
    for (vertices, normal, [u, v, uw, vh]) in faces {
        let first = positions.len() as u32;
        positions.extend(vertices.map(|p| ((Vec3::from_array(p) + center) * PIXEL).to_array()));
        normals.extend([normal.to_array(); 4]);
        uvs.extend(
            [[u, v + vh], [u + uw, v + vh], [u + uw, v], [u, v]]
                .map(|p| ((Vec2::from_array(p) + uv) / texture).to_array()),
        );
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U16(
        indices.into_iter().map(|index| index as u16).collect(),
    ));
    mesh.final_aabb = Some(Aabb3d::from_min_max(
        (center - half) * PIXEL,
        (center + half) * PIXEL,
    ));
    mesh
}

/// Model-space size of each body part: head, body, right/left arm, right/left leg.
fn part_size(part: usize, slim: bool) -> Vec3 {
    match part {
        0 => Vec3::splat(8.),
        1 => Vec3::new(8., 12., 4.),
        2 | 3 if slim => Vec3::new(3., 12., 4.),
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
            part_size(part, false),
            part_center(part),
            UV[part],
            Vec2::new(64., 32.),
            inflate,
        ))
    })
}

pub(super) fn skin_parts(meshes: &mut Assets<Mesh>, slim: bool) -> [[Handle<Mesh>; 6]; 2] {
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
                part_size(part, slim),
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
