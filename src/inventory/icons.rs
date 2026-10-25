use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDataOrder, TextureDimension, TextureFormat},
    tasks::{ComputeTaskPool, ParallelSliceMut},
};

use crate::{
    inventory::ItemKind,
    voxel::{TexturePack, VoxelAssets, VoxelKind, VoxelMaterial},
};

/// Pixels per side of a rendered icon; the UI draws them at half size.
const ICON_SIZE: usize = 64;
const ICON_BYTES: usize = ICON_SIZE * ICON_SIZE * 4;
const ICON_COUNT: usize = VoxelKind::ITEMIZABLE.len();
// Icons are stacked vertically so each one is a contiguous slice of the atlas.
const _: () = assert!(
    ICON_SIZE * ICON_COUNT <= 8192,
    "icon atlas exceeds texture limits"
);

/// Atlas index of each `VoxelKind`; only itemizable kinds are meaningful.
const ICON_INDICES: [usize; VoxelKind::ALL.len()] = {
    let mut indices = [0; VoxelKind::ALL.len()];
    let mut index = 0;
    while index < ICON_COUNT {
        indices[VoxelKind::ITEMIZABLE[index] as usize] = index;
        index += 1;
    }
    indices
};

/// Isometric projection of the cube axes, the hexagon spanning [-1, 1] vertically.
const ISO_X: f32 = 0.866_025_4;
const ISO_Y: f32 = 0.5;
/// Leaves a little room around the hexagon.
const ICON_EXTENT: f32 = 1.04;
/// Brightness of the top, left (+Z) and right (+X) faces, as in Minecraft's inventory.
const FACE_SHADES: [f32; 3] = [1.0, 0.8, 0.6];
/// Faces drawn by an icon as (axis, positive), in `FACE_SHADES` order.
const ICON_FACES: [(usize, bool); 3] = [(1, true), (2, true), (0, true)];
/// Tone closing leaf cutouts, matching `voxel_material.wesl`.
const CUTOUT_TONE: f32 = 0.32;

/// Isometric block icons rendered from the active texture pack.
#[derive(Resource, Default)]
pub(crate) struct BlockIcons {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
    /// Block texture the atlas was rendered from.
    source: Option<AssetId<Image>>,
}

impl BlockIcons {
    pub(crate) fn image(&self) -> &Handle<Image> {
        &self.image
    }

    pub(crate) fn atlas(&self, item: ItemKind) -> TextureAtlas {
        let ItemKind::Block(kind) = item;
        TextureAtlas {
            layout: self.layout.clone(),
            index: ICON_INDICES[kind as usize],
        }
    }
}

/// Allocates the atlas, transparent until the block texture is loaded.
pub(crate) fn setup_block_icons(
    mut icons: ResMut<BlockIcons>,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let size = Extent3d {
        width: ICON_SIZE as u32,
        height: (ICON_SIZE * ICON_COUNT) as u32,
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
        1,
        ICON_COUNT as u32,
        None,
        None,
    ));
    icons.source = None;
}

/// Renders the atlas once the block texture is loaded, again when the pack changes.
pub(crate) fn render_block_icons(
    mut icons: ResMut<BlockIcons>,
    mut events: MessageReader<AssetEvent<Image>>,
    voxel_assets: Res<VoxelAssets>,
    materials: Res<Assets<VoxelMaterial>>,
    texture_pack: Res<TexturePack>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(source) = materials
        .get(&voxel_assets.terrain)
        .map(|material| material.blocks.id())
    else {
        return;
    };
    let modified = events
        .read()
        .any(|event| matches!(event, AssetEvent::Modified { id } if *id == source));
    if icons.source == Some(source) && !modified {
        return;
    }
    // Still loading: retried next frame.
    let Some(blocks) = images.get(source) else {
        return;
    };
    let pixels = render_icons(blocks, &texture_pack);
    icons.source = Some(source);
    let Some(pixels) = pixels else {
        return;
    };
    if let Some(mut atlas) = images.get_mut(&icons.image) {
        atlas.data = Some(pixels);
    }
}

struct BlockTexture<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    /// Bytes from one layer to the next: layer-major data interleaves each layer's mips.
    layer_stride: usize,
    /// sRGB bytes decoded to linear.
    decode: [f32; 256],
}

impl BlockTexture<'_> {
    /// Linear color of a texel, cutouts closed as the terrain shader does.
    fn sample(&self, layer: u32, uv: Vec2) -> Vec3 {
        let x = ((uv.x * self.width as f32) as usize).min(self.width - 1);
        let y = ((uv.y * self.height as f32) as usize).min(self.height - 1);
        let offset = layer as usize * self.layer_stride + (y * self.width + x) * 4;
        let Some(&[r, g, b, a]) = self.data.get(offset..offset + 4) else {
            return Vec3::ZERO;
        };
        let rgb = Vec3::new(
            self.decode[r as usize],
            self.decode[g as usize],
            self.decode[b as usize],
        );
        Vec3::splat(CUTOUT_TONE).lerp(rgb, f32::from(a) / 255.0)
    }
}

fn render_icons(blocks: &Image, texture_pack: &TexturePack) -> Option<Vec<u8>> {
    let format = blocks.texture_descriptor.format;
    if !matches!(
        format,
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm
    ) {
        warn!("Unable to render block icons from a {format:?} texture");
        return None;
    }
    let size = blocks.texture_descriptor.size;
    let (width, height) = (size.width as usize, size.height as usize);
    // Mip-major data stores every layer's base level first.
    let layer_stride = match blocks.data_order {
        TextureDataOrder::LayerMajor => (0..blocks.texture_descriptor.mip_level_count)
            .map(|level| (width >> level).max(1) * (height >> level).max(1) * 4)
            .sum(),
        TextureDataOrder::MipMajor => width * height * 4,
    };
    let texture = BlockTexture {
        data: blocks.data.as_deref()?,
        width,
        height,
        layer_stride,
        decode: std::array::from_fn(|byte| {
            let value = byte as f32 / 255.0;
            if format.is_srgb() {
                Srgba::gamma_function(value)
            } else {
                value
            }
        }),
    };
    let mut pixels = vec![0; ICON_BYTES * ICON_COUNT];
    pixels.par_chunk_map_mut(ComputeTaskPool::get(), ICON_BYTES, |index, icon| {
        render_icon(&texture, texture_pack, VoxelKind::ITEMIZABLE[index], icon);
    });
    Some(pixels)
}

fn render_icon(
    texture: &BlockTexture,
    texture_pack: &TexturePack,
    kind: VoxelKind,
    icon: &mut [u8],
) {
    let faces = ICON_FACES.map(|(axis, positive)| {
        (
            texture_pack.texture_layer(kind, axis, positive),
            Vec3::from_array(texture_pack.texture_tint(kind, axis, positive)),
        )
    });
    for (pixel, rgba) in icon.chunks_exact_mut(4).enumerate() {
        let screen = (Vec2::new(
            (pixel % ICON_SIZE) as f32 + 0.5,
            (pixel / ICON_SIZE) as f32 + 0.5,
        ) / ICON_SIZE as f32
            * 2.0
            - 1.0)
            * ICON_EXTENT;
        let Some((face, uv)) = project(screen) else {
            continue;
        };
        let (layer, tint) = faces[face];
        let color = texture.sample(layer, uv) * tint * FACE_SHADES[face];
        let srgb = color.map(|value| Srgba::gamma_function_inverse(value.clamp(0.0, 1.0)));
        rgba.copy_from_slice(&[
            (srgb.x * 255.0).round() as u8,
            (srgb.y * 255.0).round() as u8,
            (srgb.z * 255.0).round() as u8,
            u8::MAX,
        ]);
    }
}

/// Visible face under a screen point (y down) and its texture coordinates.
///
/// Cube coordinates are centered: X goes right-down, Z left-down and Y up,
/// with the same UV orientation as the terrain mesh.
fn project(screen: Vec2) -> Option<(usize, Vec2)> {
    let inside = |a: f32, b: f32| a.abs() <= 0.5 && b.abs() <= 0.5;

    // Top (Y = 0.5): u follows Z and v follows X.
    let (sx, sy) = (screen.x / ISO_X, (screen.y + 0.5) / ISO_Y);
    let (x, z) = ((sx + sy) * 0.5, (sy - sx) * 0.5);
    if inside(x, z) {
        return Some((0, Vec2::new(z, x) + 0.5));
    }

    // Left (Z = 0.5): u follows X.
    let x = (screen.x + 0.5 * ISO_X) / ISO_X;
    let y = ISO_Y * x - (screen.y - 0.5 * ISO_Y);
    if inside(x, y) {
        return Some((1, Vec2::new(x + 0.5, 0.5 - y)));
    }

    // Right (X = 0.5): u follows Z.
    let z = -(screen.x - 0.5 * ISO_X) / ISO_X;
    let y = ISO_Y * z - (screen.y - 0.5 * ISO_Y);
    inside(z, y).then(|| (2, Vec2::new(z + 0.5, 0.5 - y)))
}
