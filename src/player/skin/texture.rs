//! Normalize legacy/HD skins and enforce vanilla base-layer opacity once per load.
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub(super) fn normalize(source: &Image) -> Result<Image, &'static str> {
    let width = source.width() as usize;
    let height = source.height() as usize;
    if width < 64 || width > 2048 || width % 64 != 0 || ![width, width / 2].contains(&height) {
        return Err(
            "Skin must be 64x64, 64x32, or a scaled version of these layouts (up to 2048 pixels).",
        );
    }
    let source = source
        .convert(TextureFormat::Rgba8UnormSrgb)
        .ok_or("Unsupported skin image format.")?;
    let input = source.data.as_ref().ok_or("Skin pixels are unavailable.")?;
    let scale = width / 64;
    let legacy = height != width;
    let mut data = vec![0; width * width * 4];
    data[..input.len()].copy_from_slice(input);
    if legacy {
        // Mirror the old right limbs into the independent left-limb regions.
        for (sx, sy, dx, dy) in [(0, 16, 16, 48), (40, 16, 32, 48)] {
            for (u, v, du, dv, w, h) in [
                (4, 0, 4, 0, 4, 4),
                (8, 0, 8, 0, 4, 4),
                (0, 4, 8, 4, 4, 12),
                (4, 4, 4, 4, 4, 12),
                (8, 4, 0, 4, 4, 12),
                (12, 4, 12, 4, 4, 12),
            ] {
                for y in 0..h * scale {
                    for x in 0..w * scale {
                        let src =
                            (((sy + v) * scale + y) * width + (sx + u) * scale + w * scale - 1 - x)
                                * 4;
                        let dst = (((dy + dv) * scale + y) * width + (dx + du) * scale + x) * 4;
                        let pixel: [u8; 4] = data[src..src + 4].try_into().unwrap();
                        data[dst..dst + 4].copy_from_slice(&pixel);
                    }
                }
            }
        }
        let hat_opaque = (0..32 * scale)
            .all(|y| (32 * scale..width).all(|x| data[(y * width + x) * 4 + 3] >= 128));
        if hat_opaque {
            for y in 0..32 * scale {
                for x in 32 * scale..width {
                    data[(y * width + x) * 4 + 3] = 0;
                }
            }
        }
    }
    for (x, y, w, h) in [(0, 0, 32, 16), (0, 16, 64, 16), (16, 48, 32, 16)] {
        for yy in y * scale..(y + h) * scale {
            for xx in x * scale..(x + w) * scale {
                data[(yy * width + xx) * 4 + 3] = 255;
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: width as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    Ok(image)
}
