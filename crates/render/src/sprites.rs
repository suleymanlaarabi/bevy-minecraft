//! Extrude alpha-tested sprites, emitting side geometry only along their silhouette.
use bevy::{asset::RenderAssetUsages, mesh::PrimitiveTopology, prelude::*};
use hollow_core::shape::BlockBox;

/// Front/back faces and silhouette edges; never extrude transparent pixels.
pub fn extrude_sprite(
    image: &Image,
    offset: Vec2,
    scale: Vec2,
    depth: f32,
    edge_shade: Vec2,
) -> Option<Mesh> {
    image.data.as_ref()?;
    let size = (image.size().as_vec2() * scale).as_uvec2();
    if size.min_element() == 0 {
        return None;
    }
    let origin = (image.size().as_vec2() * offset).as_uvec2();
    let mut opaque = Vec::with_capacity((size.x * size.y) as usize);
    for y in 0..size.y {
        for x in 0..size.x {
            opaque.push(image.get_color_at(origin.x + x, origin.y + y).ok()?.alpha() >= 0.5);
        }
    }
    let solid = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && x < size.x as i32
            && y < size.y as i32
            && opaque[(y as u32 * size.x + x as u32) as usize]
    };
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let mut quad = |points: [Vec3; 4], normal: Vec3, uv: [Vec2; 4]| {
        let first = positions.len() as u32;
        positions.extend(points.map(|point| point.to_array()));
        normals.extend([normal.to_array(); 4]);
        uvs.extend(uv.map(|uv| (offset + uv * scale).to_array()));
        let shade = 1. - edge_shade.x * normal.x.abs() - edge_shade.y * normal.y.abs();
        colors.extend([[shade, shade, shade, 1.]; 4]);
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    };
    for face in (BlockBox {
        min: Vec3::new(-0.5, -0.5, -depth),
        max: Vec3::new(0.5, 0.5, depth),
    })
    .faces()
    .filter(|face| face.axis == 2)
    {
        quad(
            face.positions,
            face.normal,
            face.positions.map(|p| Vec2::new(p.x + 0.5, 0.5 - p.y)),
        );
    }
    for y in 0..size.y as i32 {
        for x in 0..size.x as i32 {
            if !solid(x, y) {
                continue;
            }
            let left = x as f32 / size.x as f32 - 0.5;
            let right = (x + 1) as f32 / size.x as f32 - 0.5;
            let top = 0.5 - y as f32 / size.y as f32;
            let bottom = 0.5 - (y + 1) as f32 / size.y as f32;
            let uv = [Vec2::new(
                (x as f32 + 0.5) / size.x as f32,
                (y as f32 + 0.5) / size.y as f32,
            ); 4];
            for face in (BlockBox {
                min: Vec3::new(left, bottom, -depth),
                max: Vec3::new(right, top, depth),
            })
            .faces()
            .filter(|face| face.axis != 2)
            {
                if !solid(x + face.normal.x as i32, y - face.normal.y as i32) {
                    quad(face.positions, face.normal, uv);
                }
            }
        }
    }
    let vertex_count = positions.len();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(crate::model_mesh::compact_indices(indices, vertex_count));
    mesh.final_aabb = Some(bevy::shape::Aabb3d::from_min_max(
        Vec3::new(-0.5, -0.5, -depth),
        Vec3::new(0.5, 0.5, depth),
    ));
    Some(mesh)
}
