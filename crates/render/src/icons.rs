//! CPU model previews, rasterized into an atlas without a rendering-world dependency.
use crate::{RenderSource, models::ModelQuad};
use bevy::{prelude::*, render::render_resource::TextureDataOrder};
use hollow_core::StateId;
const ISO_X: f32 = 0.866_025_4;

struct BlockTexture<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
    decode: [f32; 256],
}
impl BlockTexture<'_> {
    fn sample(&self, layer: u32, uv: Vec2) -> Vec4 {
        // Model endpoints at 1 are the last texel, rather than the first texel of a repeated face.
        let x = (uv.x.clamp(0.0, 0.999999) * self.width as f32) as usize;
        let y = (uv.y.clamp(0.0, 0.999999) * self.height as f32) as usize;
        let offset = layer as usize * self.stride + (y * self.width + x) * 4;
        let Some(&[r, g, b, a]) = self.data.get(offset..offset + 4) else {
            return Vec4::ZERO;
        };
        Vec4::new(
            self.decode[r as usize],
            self.decode[g as usize],
            self.decode[b as usize],
            a as f32 / 255.0,
        )
    }
}
fn image_texture(blocks: &Image) -> BlockTexture<'_> {
    let size = blocks.texture_descriptor.size;
    let (width, height) = (size.width as usize, size.height as usize);
    let stride = match blocks.data_order {
        TextureDataOrder::LayerMajor => (0..blocks.texture_descriptor.mip_level_count)
            .map(|level| (width >> level).max(1) * (height >> level).max(1) * 4)
            .sum(),
        TextureDataOrder::MipMajor => width * height * 4,
    };
    BlockTexture {
        data: blocks.data.as_deref().unwrap_or(&[]),
        width,
        height,
        stride,
        decode: std::array::from_fn(|byte| {
            let value = byte as f32 / 255.0;
            if blocks.texture_descriptor.format.is_srgb() {
                Srgba::gamma_function(value)
            } else {
                value
            }
        }),
    }
}
pub fn rasterize_icons<C: RenderSource>(
    source: &C,
    states: impl ExactSizeIterator<Item = (StateId, Vec3)>,
    blocks: &Image,
    special: Option<&Image>,
    table: &[[f32; 4]],
    tile_size: usize,
    columns: usize,
) -> Vec<u8> {
    let texture = image_texture(blocks);
    let special = special.map(image_texture);
    let rows = states.len().div_ceil(columns);
    let grid_width = columns * tile_size;
    let mut grid = vec![0; (tile_size * tile_size * 4) * columns * rows];
    let mut icon = vec![Vec4::ZERO; tile_size * tile_size];
    let mut depths = vec![f32::NEG_INFINITY; tile_size * tile_size];
    let mut quads = Vec::new();
    for (index, (state, tint)) in states.enumerate() {
        icon.fill(Vec4::ZERO);
        depths.fill(f32::NEG_INFINITY);
        render_icon(
            &texture,
            special.as_ref(),
            table,
            source,
            state,
            tint,
            tile_size,
            &mut quads,
            &mut icon,
            &mut depths,
        );
        for (pixel, &color) in icon.iter().enumerate() {
            let offset = (((index / columns) * tile_size + pixel / tile_size) * grid_width
                + (index % columns) * tile_size
                + pixel % tile_size)
                * 4;
            let rgb = color
                .truncate()
                .map(|v| Srgba::gamma_function_inverse(v.clamp(0.0, 1.0)));
            grid[offset..offset + 4].copy_from_slice(&[
                (rgb.x * 255.0).round() as u8,
                (rgb.y * 255.0).round() as u8,
                (rgb.z * 255.0).round() as u8,
                (color.w * 255.0).round() as u8,
            ]);
        }
    }
    grid
}
fn render_icon<C: RenderSource>(
    texture: &BlockTexture,
    special: Option<&BlockTexture>,
    table: &[[f32; 4]],
    source: &C,
    state: StateId,
    tint: Vec3,
    tile_size: usize,
    quads: &mut Vec<ModelQuad>,
    pixels: &mut [Vec4],
    depths: &mut [f32],
) {
    quads.clear();
    source.visit_quads(state, IVec3::ZERO, |q| quads.push(q.clone()));
    let (min, max) = quads
        .iter()
        .flat_map(|q| q.positions)
        .map(Vec3::from_array)
        .fold((Vec3::splat(0.5), Vec3::splat(0.5)), |(min, max), p| {
            (min.min(p), max.max(p))
        });
    let center = (min + max) * 0.5;
    let scale = (max - min).max_element().max(1.0);
    let eye = Vec3::ONE.normalize();
    quads.sort_unstable_by(|a, b| {
        let depth = |q: &ModelQuad| {
            q.positions
                .iter()
                .map(|&p| Vec3::from_array(p).dot(eye))
                .sum::<f32>()
        };
        depth(a).total_cmp(&depth(b))
    });
    for q in quads.iter() {
        let normal = Vec3::from_array(q.normal);
        if normal.dot(eye) <= 0.0001 {
            continue;
        }
        let Some(face) = table.get(q.texture as usize) else {
            continue;
        };
        let tint = Vec3::new(face[0], face[1], face[2]) * if q.tinted { tint } else { Vec3::ONE };
        let shade = if q.shade_up {
            1.0
        } else {
            normal.abs().dot(Vec3::new(0.6, 1.0, 0.8)).min(1.0)
        };
        let points = q.positions.map(|p| {
            let p = (Vec3::from_array(p) - center) / scale;
            Vec3::new(
                (ISO_X * (p.x - p.z) / 1.04 + 1.0) * 0.5 * tile_size as f32,
                ((0.5 * (p.x + p.z) - p.y) / 1.04 + 1.0) * 0.5 * tile_size as f32,
                p.dot(eye),
            )
        });
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            let [a, b, c] = triangle.map(|i| points[i]);
            let [ua, ub, uc] = triangle.map(|i| Vec2::from_array(q.uv[i]));
            let edge = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
            let area = edge(a.truncate(), b.truncate(), c.truncate());
            if area.abs() < 1e-6 {
                continue;
            }
            let min = a
                .min(b)
                .min(c)
                .truncate()
                .floor()
                .max(Vec2::ZERO)
                .as_uvec2();
            let max = a
                .max(b)
                .max(c)
                .truncate()
                .ceil()
                .min(Vec2::splat(tile_size as f32))
                .max(Vec2::ZERO)
                .as_uvec2();
            for y in min.y..max.y {
                for x in min.x..max.x {
                    let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = edge(b.truncate(), c.truncate(), p) / area;
                    let w1 = edge(c.truncate(), a.truncate(), p) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let index = y as usize * tile_size + x as usize;
                    let z = a.z * w0 + b.z * w1 + c.z * w2;
                    if z <= depths[index] + 1e-6 {
                        continue;
                    }
                    let layer = face[3] as u32;
                    let bank = if layer & 4096 != 0 {
                        special.unwrap_or(texture)
                    } else {
                        texture
                    };
                    let color = bank.sample(layer & 2047, ua * w0 + ub * w1 + uc * w2);
                    if color.w < if q.translucent { 0.004 } else { 0.5 } {
                        continue;
                    }
                    let alpha = if q.translucent { color.w } else { 1.0 };
                    let rgb = color.truncate() * tint * shade;
                    let old = pixels[index];
                    let out_alpha = alpha + old.w * (1.0 - alpha);
                    let out_rgb = (rgb * alpha + old.truncate() * old.w * (1.0 - alpha))
                        / out_alpha.max(1e-6);
                    pixels[index] = out_rgb.extend(out_alpha);
                    depths[index] = z;
                }
            }
        }
    }
}
