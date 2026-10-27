use super::material::VoxelMaterial;
use bevy::{
    prelude::*,
    render::render_resource::{TextureDataOrder, TextureFormat},
    tasks::{Task, futures::check_ready},
};

/// Box-filtered mip chain of a pixel-art array texture, as `(data, level count)` in
/// layer-major order, or `None` when the image already has one or is unsupported.
///
/// Without it, distant terrain samples full-resolution texels: they shimmer and
/// scatter texture reads across the cache. Colors are averaged in linear space and
/// weighted by alpha. The terrain shader shades a texel as `mix(dark, rgb, alpha)`,
/// which is linear in `(rgb * alpha, alpha)`, so each level is exactly the average of
/// the shaded texels it covers and leaf cutouts keep their brightness at a distance.
fn block_mipmaps(image: &Image) -> Option<(Vec<u8>, u32)> {
    let levels = mipmap_levels(image)?;
    let descriptor = &image.texture_descriptor;
    let srgb = descriptor.format == TextureFormat::Rgba8UnormSrgb;
    let (width, height) = (descriptor.size.width, descriptor.size.height);
    let data = image.data.as_ref()?;
    let layer_bytes = (width * height * 4) as usize;

    let decode: [f32; 256] = std::array::from_fn(|value| {
        let value = value as f32 / 255.0;
        if !srgb {
            value
        } else if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    let encode = |value: f32| {
        let value = value.clamp(0.0, 1.0);
        let value = if !srgb {
            value
        } else if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (value * 255.0).round() as u8
    };

    let mut mipmapped = Vec::with_capacity(data.len() * 4 / 3 + 4);
    for layer in data.chunks_exact(layer_bytes) {
        mipmapped.extend_from_slice(layer);
        // Premultiplied linear color: plain averages are then correct.
        let mut texels: Vec<[f32; 4]> = layer
            .chunks_exact(4)
            .map(|texel| {
                let alpha = f32::from(texel[3]) / 255.0;
                [
                    decode[texel[0] as usize] * alpha,
                    decode[texel[1] as usize] * alpha,
                    decode[texel[2] as usize] * alpha,
                    alpha,
                ]
            })
            .collect();
        let (mut level_width, mut level_height) = (width as usize, height as usize);
        for _ in 1..levels {
            let (next_width, next_height) = (level_width / 2, level_height / 2);
            texels = (0..next_width * next_height)
                .map(|index| {
                    let (x, y) = (index % next_width * 2, index / next_width * 2);
                    let mut sum = [0.0; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let texel = texels[x + dx + (y + dy) * level_width];
                        for channel in 0..4 {
                            sum[channel] += texel[channel] * 0.25;
                        }
                    }
                    sum
                })
                .collect();
            mipmapped.extend(texels.iter().flat_map(|&[r, g, b, alpha]| {
                let color = |value: f32| encode(if alpha > 0.0 { value / alpha } else { 0.0 });
                [color(r), color(g), color(b), (alpha * 255.0).round() as u8]
            }));
            (level_width, level_height) = (next_width, next_height);
        }
    }
    Some((mipmapped, levels))
}

/// Mip levels [`block_mipmaps`] would build, or `None` when the image already has
/// them or is unsupported. Cheap: only the descriptor and the data size are read.
fn mipmap_levels(image: &Image) -> Option<u32> {
    let descriptor = &image.texture_descriptor;
    if !matches!(
        descriptor.format,
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm
    ) {
        return None;
    }
    let (width, height) = (descriptor.size.width, descriptor.size.height);
    if descriptor.mip_level_count != 1 || !width.is_power_of_two() || !height.is_power_of_two() {
        return None;
    }
    let levels = width.min(height).ilog2() + 1;
    let layer_bytes = (width * height * 4) as usize;
    let expected = layer_bytes * descriptor.size.depth_or_array_layers as usize;
    (levels > 1 && image.data.as_ref()?.len() == expected).then_some(levels)
}

/// Mip chain being computed off the main thread for one block texture.
#[derive(Resource)]
pub(crate) struct MipmapJob {
    image: AssetId<Image>,
    task: Task<Option<(Vec<u8>, u32)>>,
}

/// Gives the terrain's block texture its mip chain once it loads, including after a
/// texture pack switch. Runs on image or material asset events, and while a job runs.
pub(crate) fn generate_block_mipmaps(
    budget: Res<hollow_worldgen::tasks::VoxelTaskBudget>,
    mut commands: Commands,
    assets: Option<Res<super::assets::VoxelAssets>>,
    materials: Res<Assets<VoxelMaterial>>,
    mut images: ResMut<Assets<Image>>,
    job: Option<ResMut<MipmapJob>>,
) {
    let Some(source) = assets
        .and_then(|assets| materials.get(&assets.terrain))
        .map(|material| material.blocks.id())
    else {
        return;
    };
    if let Some(mut job) = job {
        let Some(mipmapped) = check_ready(&mut job.task) else {
            return;
        };
        commands.remove_resource::<MipmapJob>();
        // A pack switch during the job leaves the result unused: the new texture is
        // picked up below. Checked before `get_mut`, which re-uploads the image.
        if job.image == source
            && let Some((data, levels)) = mipmapped
            && images.get(source).and_then(mipmap_levels) == Some(levels)
            && let Some(mut image) = images.get_mut(source)
        {
            image.data = Some(data);
            image.data_order = TextureDataOrder::LayerMajor;
            image.texture_descriptor.mip_level_count = levels;
            return;
        }
    }
    let Some(image) = images
        .get(source)
        .filter(|image| mipmap_levels(image).is_some())
    else {
        return;
    };
    let image = image.clone();
    commands.insert_resource(MipmapJob {
        image: source,
        task: budget.spawn(hollow_worldgen::tasks::WorkClass::Loading, 0, async move {
            block_mipmaps(&image)
        }),
    });
}
