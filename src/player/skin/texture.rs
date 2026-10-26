//! Enforce the default skin layout and base-layer opacity once per load.
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub(super) fn normalize(source: &Image) -> Result<Image, &'static str> {
    let width = source.width() as usize;
    let height = source.height() as usize;
    if width != 64 || height != 64 {
        return Err("Default skin must use the 64x64 layout.");
    }
    let source = source
        .convert(TextureFormat::Rgba8UnormSrgb)
        .ok_or("Unsupported skin image format.")?;
    let input = source.data.as_ref().ok_or("Skin pixels are unavailable.")?;
    let mut data = input.clone();
    for (x, y, w, h) in [(0, 0, 32, 16), (0, 16, 64, 16), (16, 48, 32, 16)] {
        for yy in y..y + h {
            for xx in x..x + w {
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
