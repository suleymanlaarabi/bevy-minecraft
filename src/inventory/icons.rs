//! Inventory models are rasterized once per texture revision, off the ECS thread.
use crate::{
    inventory::ItemKind,
    voxel::{TexturePack, VoxelAssets, VoxelKind, VoxelMaterial, render::models},
};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDataOrder, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

const ICON_SIZE: usize = 64;
const ICON_BYTES: usize = ICON_SIZE * ICON_SIZE * 4;
const ICON_COLUMNS: usize = 32;
const ISO_X: f32 = 0.866_025_4;
fn icon_count() -> usize {
    VoxelKind::itemizable().len()
}
fn icon_index(kind: VoxelKind) -> usize {
    static INDICES: std::sync::OnceLock<Vec<usize>> = std::sync::OnceLock::new();
    INDICES.get_or_init(|| {
        let mut indices = vec![0; VoxelKind::ALL.len()];
        for (index, &kind) in VoxelKind::itemizable().iter().enumerate() {
            indices[kind.index()] = index;
        }
        indices
    })[kind.index()]
}
struct IconOutput {
    revision: u64,
    pixels: Vec<u8>,
}
#[derive(Resource, Default)]
pub(crate) struct BlockIcons {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
    source: Option<AssetId<Image>>,
    revision: u64,
    finished: Option<u64>,
    task: Option<Task<IconOutput>>,
}
impl BlockIcons {
    pub(crate) fn image(&self) -> &Handle<Image> {
        &self.image
    }
    pub(crate) fn atlas(&self, item: ItemKind) -> TextureAtlas {
        let ItemKind::Block(kind) = item;
        TextureAtlas {
            layout: self.layout.clone(),
            index: icon_index(kind),
        }
    }
}
pub(crate) fn setup_block_icons(
    mut icons: ResMut<BlockIcons>,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let rows = icon_count().div_ceil(ICON_COLUMNS);
    let size = Extent3d {
        width: (ICON_SIZE * ICON_COLUMNS) as u32,
        height: (ICON_SIZE * rows) as u32,
        depth_or_array_layers: 1,
    };
    icons.image = images.add(Image::new_fill(
        size,
        TextureDimension::D2,
        &[0; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ));
    icons.layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(ICON_SIZE as u32),
        ICON_COLUMNS as u32,
        rows as u32,
        None,
        None,
    ));
}
pub(crate) fn render_block_icons(
    mut icons: ResMut<BlockIcons>,
    mut events: MessageReader<AssetEvent<Image>>,
    voxel_assets: Res<VoxelAssets>,
    materials: Res<Assets<VoxelMaterial>>,
    texture_pack: Res<TexturePack>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(source) = materials.get(&voxel_assets.terrain).map(|m| m.blocks.id()) else {
        return;
    };
    let modified = events
        .read()
        .any(|e| matches!(e,AssetEvent::Modified{id} if *id==source));
    if icons.source != Some(source) || modified {
        icons.source = Some(source);
        icons.revision = icons.revision.wrapping_add(1);
        icons.task = None;
    }
    if let Some(task) = icons.task.as_mut()
        && let Some(output) = check_ready(task)
    {
        icons.task = None;
        if output.revision == icons.revision {
            if let Some(mut image) = images.get_mut(&icons.image) {
                image.data = Some(output.pixels);
            }
            icons.finished = Some(output.revision);
        }
    }
    if icons.task.is_some() || icons.finished == Some(icons.revision) {
        return;
    }
    let Some(blocks) = images.get(source) else {
        return;
    };
    if !matches!(
        blocks.texture_descriptor.format,
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm
    ) || blocks.data.is_none()
    {
        return;
    }
    let blocks = blocks.clone();
    let table = texture_pack.model_face_table();
    let revision = icons.revision;
    icons.task = Some(AsyncComputeTaskPool::get().spawn(async move {
        IconOutput {
            revision,
            pixels: render_icons(&blocks, &table),
        }
    }));
}
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
fn render_icons(blocks: &Image, table: &[[f32; 4]]) -> Vec<u8> {
    let size = blocks.texture_descriptor.size;
    let (width, height) = (size.width as usize, size.height as usize);
    let stride = match blocks.data_order {
        TextureDataOrder::LayerMajor => (0..blocks.texture_descriptor.mip_level_count)
            .map(|level| (width >> level).max(1) * (height >> level).max(1) * 4)
            .sum(),
        TextureDataOrder::MipMajor => width * height * 4,
    };
    let texture = BlockTexture {
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
    };
    let rows = icon_count().div_ceil(ICON_COLUMNS);
    let grid_width = ICON_COLUMNS * ICON_SIZE;
    let mut grid = vec![0; ICON_BYTES * ICON_COLUMNS * rows];
    let mut icon = vec![Vec4::ZERO; ICON_SIZE * ICON_SIZE];
    let mut depths = vec![f32::NEG_INFINITY; ICON_SIZE * ICON_SIZE];
    for (index, &kind) in VoxelKind::itemizable().iter().enumerate() {
        icon.fill(Vec4::ZERO);
        depths.fill(f32::NEG_INFINITY);
        render_icon(&texture, table, kind, &mut icon, &mut depths);
        for (pixel, &color) in icon.iter().enumerate() {
            let offset = (((index / ICON_COLUMNS) * ICON_SIZE + pixel / ICON_SIZE) * grid_width
                + (index % ICON_COLUMNS) * ICON_SIZE
                + pixel % ICON_SIZE)
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
fn render_icon(
    texture: &BlockTexture,
    table: &[[f32; 4]],
    kind: VoxelKind,
    pixels: &mut [Vec4],
    depths: &mut [f32],
) {
    let mut quads = Vec::new();
    models::visit_quads(kind.default_state(), IVec3::ZERO, |q| quads.push(q.clone()));
    let eye = Vec3::ONE.normalize();
    quads.sort_unstable_by(|a, b| {
        let depth = |q: &models::ModelQuad| {
            q.positions
                .iter()
                .map(|&p| Vec3::from_array(p).dot(eye))
                .sum::<f32>()
        };
        depth(a).total_cmp(&depth(b))
    });
    for q in quads {
        let normal = Vec3::from_array(q.normal);
        if normal.dot(eye) <= 0.0001 {
            continue;
        }
        let Some(face) = table.get(q.texture as usize) else {
            continue;
        };
        let tint = Vec3::new(face[0], face[1], face[2])
            * if q.tinted {
                Vec3::from_array(models::tint(kind))
            } else {
                Vec3::ONE
            };
        let shade = if q.shade_up {
            1.0
        } else {
            normal.abs().dot(Vec3::new(0.6, 1.0, 0.8)).min(1.0)
        };
        let points = q.positions.map(|p| {
            let p = Vec3::from_array(p) - Vec3::splat(0.5);
            Vec3::new(
                (ISO_X * (p.x - p.z) / 1.04 + 1.0) * 0.5 * ICON_SIZE as f32,
                ((0.5 * (p.x + p.z) - p.y) / 1.04 + 1.0) * 0.5 * ICON_SIZE as f32,
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
                .min(Vec2::splat(ICON_SIZE as f32))
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
                    let index = y as usize * ICON_SIZE + x as usize;
                    let z = a.z * w0 + b.z * w1 + c.z * w2;
                    if z <= depths[index] + 1e-6 {
                        continue;
                    }
                    let color = texture.sample(face[3] as u32, ua * w0 + ub * w1 + uc * w2);
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
