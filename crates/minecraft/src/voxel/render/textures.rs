use crate::voxel::VoxelKind;
use bevy::{
    asset::RenderAssetUsages, image::ImageSampler, prelude::*, render::storage::ShaderBuffer,
};
use serde::Deserialize;
use std::collections::HashMap;

const BLOCK_TEXTURE_PATH: &str = "voxel/packs/vanilla/blocks_array.png";

fn white() -> (f32, f32, f32) {
    (1.0, 1.0, 1.0)
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct BlockTextureConfig {
    top: u32,
    side: u32,
    bottom: u32,
    #[serde(default = "white")]
    tint_top: (f32, f32, f32),
    #[serde(default = "white")]
    tint_side: (f32, f32, f32),
    #[serde(default = "white")]
    tint_bottom: (f32, f32, f32),
}

/// Text keys exist only while deserializing the manifest.
type BlockTextures = HashMap<String, BlockTextureConfig>;

#[derive(Debug, Deserialize)]
struct TextureManifest {
    layers: u32,
    blocks: BlockTextures,
}

#[derive(Resource, Debug)]
pub(crate) struct MinecraftTextures {
    layers: u32,
    /// Fallbacks resolved at load, indexed by `VoxelKind as usize`; `None` when not opaque.
    blocks: Box<[Option<BlockTextureConfig>]>,
}

/// Blocks still recognizable when drawn with their fallback's texture, tinted by their color.
const TINTED_FALLBACKS: [VoxelKind; 1] = [VoxelKind::MagmaBlock];

fn resolve(blocks: &[Option<BlockTextureConfig>], kind: VoxelKind) -> Option<BlockTextureConfig> {
    let state = kind.default_state();
    let fluid = state.definition().fluid.id;
    if state.is_air()
        || (fluid != bevy_voxel::core::FluidId::NONE
            && crate::content::fluid_catalog()
                .profile(fluid)
                .render
                .is_some_and(|render| render.ocean_surface))
    {
        return None;
    }
    let mut source = kind;
    let mut config = loop {
        if let Some(config) = blocks[source.index()].as_ref() {
            break *config;
        }
        let fallback = source.fallback();
        if fallback == source {
            break blocks[VoxelKind::Stone.index()].expect("Stone fallback");
        }
        source = fallback;
    };
    if source != kind
        && (TINTED_FALLBACKS.contains(&kind) || fluid != bevy_voxel::core::FluidId::NONE)
    {
        let [r, g, b, _] = kind.color();
        let [base_r, base_g, base_b, _] = source.color();
        let tint = (
            (r / base_r).min(2.0),
            (g / base_g).min(2.0),
            (b / base_b).min(2.0),
        );
        (config.tint_top, config.tint_side, config.tint_bottom) = (tint, tint, tint);
    }
    Some(config)
}

impl TextureManifest {
    fn resolved_blocks(&self) -> Result<Box<[Option<BlockTextureConfig>]>, String> {
        let mut blocks = vec![None; VoxelKind::count()];
        for (name, &config) in &self.blocks {
            let kind = VoxelKind::from_key(name)
                .ok_or_else(|| format!("Unknown Minecraft texture block: {name}"))?;
            if blocks[kind.index()].replace(config).is_some() {
                return Err(format!("Duplicate Minecraft texture block: {name}"));
            }
        }
        if blocks[VoxelKind::Stone.index()].is_none() {
            return Err("Missing stone fallback texture".into());
        }
        Ok(blocks.into())
    }
}

/// Native block identities present in a validated manifest.
pub(crate) fn manifest_keys() -> Box<[bool]> {
    let manifest: TextureManifest = ron::from_str(include_str!(
        "../../../../../assets/voxel/packs/vanilla/pack.ron"
    ))
    .expect("Invalid Minecraft texture manifest");
    manifest
        .resolved_blocks()
        .expect("Invalid Minecraft texture identities")
        .iter()
        .map(Option::is_some)
        .collect()
}

impl MinecraftTextures {
    pub(crate) fn load() -> Self {
        assert!(
            crate::asset_directory().join(BLOCK_TEXTURE_PATH).is_file(),
            "Missing Minecraft textures; run scripts/import_vanilla.py"
        );
        let manifest: TextureManifest = ron::from_str(include_str!(
            "../../../../../assets/voxel/packs/vanilla/pack.ron"
        ))
        .expect("Invalid Minecraft texture manifest");
        let blocks = manifest
            .resolved_blocks()
            .expect("Invalid Minecraft texture identities");
        Self {
            layers: manifest.layers,
            blocks: VoxelKind::all()
                .map(|kind| resolve(&blocks, kind))
                .collect(),
        }
    }

    pub(crate) fn load_texture(
        &self,
        server: &AssetServer,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        use bevy::image::{CompressedImageFormats, ImageType};
        use bevy::render::render_resource::TextureFormat;
        let external = super::models::external_textures();
        if external.is_empty() {
            return block_texture(server, BLOCK_TEXTURE_PATH, self.layers);
        }
        // Packing is initialization work, never performed by mesh workers.
        // One shared array avoids a texture/material allocation per custom block.
        let decode = |path: &str| {
            let bytes = std::fs::read(crate::asset_directory().join(path))
                .unwrap_or_else(|e| panic!("Cannot load content image {path}: {e}"));
            let image = Image::from_buffer(
                &bytes,
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::nearest(),
                RenderAssetUsages::all(),
            )
            .expect("Invalid content image");
            if image.texture_descriptor.format == TextureFormat::Rgba8UnormSrgb {
                image
            } else {
                image
                    .convert(TextureFormat::Rgba8UnormSrgb)
                    .expect("Content image must support RGBA8")
            }
        };
        let mut image = decode(BLOCK_TEXTURE_PATH);
        let width = image.width();
        let height = image.height() / self.layers;
        assert_eq!(
            image.height(),
            height * self.layers,
            "Invalid texture array height"
        );
        if self.layers > 1 {
            image
                .reinterpret_stacked_2d_as_array(self.layers)
                .expect("Invalid texture array");
        }
        let layers = self.layers + u32::try_from(external.len()).expect("Too many external images");
        assert!(
            layers <= 2048,
            "Texture array exceeds the renderer's physical layer capacity"
        );
        let pixels = image.data.as_mut().expect("CPU texture data");
        pixels.reserve(width as usize * height as usize * 4 * external.len());
        for (resource, _) in external {
            let source = decode(hollow_worldgen::content::catalog().resource(*resource));
            let data = source.data.as_ref().unwrap();
            // Nearest sampling matches Minecraft voxel textures. Source dimensions
            // are normalized once, so custom content can use a different resolution.
            for y in 0..height {
                for x in 0..width {
                    let offset = ((y * source.height() / height) * source.width()
                        + x * source.width() / width) as usize
                        * 4;
                    pixels.extend_from_slice(&data[offset..offset + 4]);
                }
            }
        }
        image.texture_descriptor.size.depth_or_array_layers = layers;
        image.sampler = bevy_voxel::render::assets::block_sampler();
        images.add(image)
    }
    fn config(&self, kind: VoxelKind) -> Option<&BlockTextureConfig> {
        self.blocks[kind.index()].as_ref()
    }

    pub(crate) fn texture_layer(self: &Self, kind: VoxelKind, axis: usize, positive: bool) -> u32 {
        let config = self
            .config(kind)
            .expect("opaque voxels have texture configuration");
        if axis == 1 {
            if positive { config.top } else { config.bottom }
        } else {
            config.side
        }
    }

    pub(crate) fn texture_tint(
        self: &Self,
        kind: VoxelKind,
        axis: usize,
        positive: bool,
    ) -> [f32; 3] {
        let config = self
            .config(kind)
            .expect("opaque voxels have texture configuration");
        let [r, g, b] = if axis == 1 {
            if positive {
                [config.tint_top.0, config.tint_top.1, config.tint_top.2]
            } else {
                [
                    config.tint_bottom.0,
                    config.tint_bottom.1,
                    config.tint_bottom.2,
                ]
            }
        } else {
            [config.tint_side.0, config.tint_side.1, config.tint_side.2]
        };
        let tint = Color::srgb(r, g, b).to_linear();
        [tint.red, tint.green, tint.blue]
    }

    /// `(tint, layer)` of every block face, indexed by `kind * 3 + slot` with slots
    /// top, side and bottom, as read by `voxel_material.wesl`.
    fn face_table(&self) -> Vec<[f32; 4]> {
        VoxelKind::all()
            .flat_map(|kind| {
                [(1, true), (0, true), (1, false)].map(|(axis, positive)| {
                    if self.config(kind).is_none() {
                        return [0.0; 4];
                    }
                    let [r, g, b] = self.texture_tint(kind, axis, positive);
                    let layer = self.texture_layer(kind, axis, positive);
                    let layer = super::models::animated_layer(layer);
                    [r, g, b, layer as f32]
                })
            })
            .collect()
    }

    /// Water samples the block array only when both imported fluid textures exist.
    pub(crate) fn surface_water(&self) -> bool {
        ["water_still", "water_flow"].into_iter().all(|name| {
            super::models::texture_names()
                .iter()
                .any(|texture| texture == name)
        })
    }

    pub(crate) fn model_face_table(&self) -> Vec<[f32; 4]> {
        let texture_count = super::models::texture_names().len();
        let external = super::models::external_textures();
        let mut table =
            Vec::with_capacity(texture_count + VoxelKind::count() * 3 + super::models::TINTS.len());
        // Custom image slots are appended after the imported Minecraft slots.
        table.extend(
            (0..texture_count - external.len())
                .map(|index| [1.0, 1.0, 1.0, super::models::texture_layer(index) as f32]),
        );
        table.resize(texture_count, [0.0; 4]);
        for (offset, &(_, slot)) in external.iter().enumerate() {
            let layer = self.layers + u32::try_from(offset).expect("Too many external images");
            table[slot as usize] = [1.0, 1.0, 1.0, layer as f32];
        }
        table.extend(self.face_table());
        // Tint palette, read from the end of the table by `model_vertex.wesl`.
        table.extend(super::models::TINTS.map(|[r, g, b]| [r, g, b, 0.0]));
        table
    }
    pub(crate) fn face_buffers(&self) -> (ShaderBuffer, ShaderBuffer) {
        let model = self.model_face_table();
        let cube = self.cube_face_table(&model);
        (
            ShaderBuffer::new(cube, RenderAssetUsages::RENDER_WORLD),
            ShaderBuffer::new(model, RenderAssetUsages::RENDER_WORLD),
        )
    }

    /// Both material families share one immutable animation descriptor buffer.
    pub(crate) fn animation_buffer(&self) -> ShaderBuffer {
        ShaderBuffer::new(
            super::models::animation_table(),
            RenderAssetUsages::RENDER_WORLD,
        )
    }

    pub(crate) fn special_texture(&self, server: &AssetServer) -> Option<Handle<Image>> {
        let layers = super::models::special_layers();
        (layers != 0)
            .then(|| block_texture(server, "voxel/packs/vanilla/special_array.png", layers))
    }

    /// `(tint, layer | uv transform << 12)` of each greedy cube appearance, six
    /// faces each indexed by direction, as read by `voxel_material.wesl`.
    fn cube_face_table(&self, model: &[[f32; 4]]) -> Vec<[f32; 4]> {
        let mut table: Vec<[f32; 4]> = super::models::appearances()
            .iter()
            .flat_map(|appearance| {
                appearance.faces.map(|face| {
                    let [r, g, b, layer] =
                        model.get(face.slot as usize).copied().unwrap_or_default();
                    let [tr, tg, tb] = if face.tinted {
                        super::models::tint(appearance.tint)
                    } else {
                        [1.0; 3]
                    };
                    [
                        r * tr,
                        g * tg,
                        b * tb,
                        layer + 4096.0 * f32::from(face.transform),
                    ]
                })
            })
            .collect();
        // Storage bindings cannot be empty.
        if table.is_empty() {
            table.resize(6, [0.0; 4]);
        }
        table
    }
}

pub(crate) use bevy_voxel::render::assets::load_block_texture as block_texture;
