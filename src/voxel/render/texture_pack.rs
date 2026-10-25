use crate::voxel::VoxelKind;
use bevy::{
    image::{
        ImageAddressMode, ImageArrayLayout, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    prelude::*,
    render::storage::ShaderBuffer,
};
use serde::Deserialize;
use std::collections::HashMap;

fn default_texture_layers() -> u32 {
    9
}

#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq)]
#[type_path = "hollow::voxel::material"]
pub enum TexturePackId {
    Vanilla,
    Faithful,
    Replete,
    Meadow,
    Twilight,
}

impl TexturePackId {
    pub const fn all() -> [Self; 5] {
        [
            Self::Vanilla,
            Self::Faithful,
            Self::Replete,
            Self::Meadow,
            Self::Twilight,
        ]
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Vanilla => "Minecraft",
            Self::Faithful => "Faithful 32x",
            Self::Replete => "Replete",
            Self::Meadow => "Meadow",
            Self::Twilight => "Twilight",
        }
    }

    /// Vanilla textures are not redistributable: they exist only after running
    /// scripts/import_vanilla.py. Every other pack ships with the game.
    pub fn is_available(self) -> bool {
        self != Self::Vanilla
            || crate::asset_directory()
                .join(format!("voxel/packs/{}/blocks_array.png", self.directory()))
                .is_file()
    }

    const fn directory(self) -> &'static str {
        match self {
            Self::Vanilla => "vanilla",
            Self::Faithful => "faithful",
            Self::Replete => "replete",
            Self::Meadow => "meadow",
            Self::Twilight => "twilight",
        }
    }

    const fn manifest_source(self) -> &'static str {
        match self {
            Self::Vanilla => include_str!("../../../assets/voxel/packs/vanilla/pack.ron"),
            Self::Faithful => include_str!("../../../assets/voxel/packs/faithful/pack.ron"),
            Self::Replete => include_str!("../../../assets/voxel/packs/replete/pack.ron"),
            Self::Meadow => include_str!("../../../assets/voxel/packs/meadow/pack.ron"),
            Self::Twilight => include_str!("../../../assets/voxel/packs/twilight/pack.ron"),
        }
    }
}

/// Minecraft textures when imported, otherwise the bundled Faithful pack.
impl Default for TexturePackId {
    fn default() -> Self {
        if Self::Vanilla.is_available() {
            Self::Vanilla
        } else {
            Self::Faithful
        }
    }
}

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

/// Textures keyed by `VoxelKind::key`; missing blocks use their fallback chain.
type BlockTextures = HashMap<String, BlockTextureConfig>;

#[derive(Debug, Deserialize)]
struct TexturePackManifest {
    name: String,
    texture: String,
    #[serde(default = "default_texture_layers")]
    layers: u32,
    blocks: BlockTextures,
}

#[derive(Resource, Clone, Debug)]
pub(crate) struct TexturePack {
    /// Pack actually loaded, after any fallback.
    id: TexturePackId,
    pub(crate) texture_path: String,
    pub(crate) layers: u32,
    /// Fallbacks resolved at load, indexed by `VoxelKind as usize`; `None` when not opaque.
    blocks: Box<[Option<BlockTextureConfig>]>,
}

/// Blocks still recognizable when drawn with their fallback's texture, tinted by their color.
const TINTED_FALLBACKS: [VoxelKind; 2] = [VoxelKind::Lava, VoxelKind::MagmaBlock];

fn resolve(blocks: &BlockTextures, kind: VoxelKind) -> Option<BlockTextureConfig> {
    if matches!(kind, VoxelKind::Air | VoxelKind::Water) {
        return None;
    }
    let mut source = kind;
    let mut config = loop {
        if let Some(config) = blocks.get(source.key()) {
            break *config;
        }
        let fallback = source.fallback();
        if fallback == source {
            break blocks[VoxelKind::Stone.key()];
        }
        source = fallback;
    };
    if source != kind && TINTED_FALLBACKS.contains(&kind) {
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

/// Block keys of a pack's manifest.
pub(crate) fn manifest_keys(id: TexturePackId) -> std::collections::HashSet<String> {
    ron::from_str::<TexturePackManifest>(id.manifest_source())
        .map(|manifest| manifest.blocks.into_keys().collect())
        .unwrap_or_default()
}

impl TexturePack {
    pub(crate) fn load(id: TexturePackId) -> Self {
        Self::try_load(id).unwrap_or_else(|error| {
            // Vanilla textures are not redistributable: they exist only after
            // running scripts/import_vanilla.py.
            warn!("Unable to load texture pack {}: {error}", id.label());
            // Bundled and checked by tests: no filesystem access can make this fail.
            Self::try_load(TexturePackId::Faithful)
                .expect("the built-in Faithful texture pack must be valid")
        })
    }

    fn try_load(id: TexturePackId) -> Result<Self, String> {
        let manifest = ron::from_str::<TexturePackManifest>(id.manifest_source())
            .map_err(|error| error.to_string())?;
        if !manifest.blocks.contains_key(VoxelKind::Stone.key()) {
            return Err("no \"stone\" entry, the fallback of every block".into());
        }
        let texture_path = format!("voxel/packs/{}/{}", id.directory(), manifest.texture);
        if !id.is_available() {
            return Err(format!("missing {texture_path}"));
        }
        debug!("Loaded texture pack {} ({})", id.label(), manifest.name);
        Ok(Self {
            id,
            texture_path,
            layers: manifest.layers,
            blocks: VoxelKind::ALL
                .iter()
                .map(|&kind| resolve(&manifest.blocks, kind))
                .collect(),
        })
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
        VoxelKind::ALL
            .iter()
            .flat_map(|&kind| {
                [(1, true), (0, true), (1, false)].map(|(axis, positive)| {
                    if self.config(kind).is_none() {
                        return [0.0; 4];
                    }
                    let [r, g, b] = self.texture_tint(kind, axis, positive);
                    [r, g, b, self.texture_layer(kind, axis, positive) as f32]
                })
            })
            .collect()
    }

    /// Whether model texture slots map one to one onto this pack's layers. Other
    /// packs remap them, so the vanilla opacity of a slot says nothing.
    pub(crate) fn vanilla_models(&self) -> bool {
        self.id == TexturePackId::Vanilla
    }

    pub(crate) fn model_face_table(&self) -> Vec<[f32; 4]> {
        let vanilla = self.vanilla_models();
        let mut table = super::models::texture_names()
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if vanilla {
                    return [1.0, 1.0, 1.0, index as f32];
                }
                let kind = VoxelKind::from_key(name)
                    .or_else(|| name.strip_suffix("_top").and_then(VoxelKind::from_key))
                    .or_else(|| name.strip_suffix("_side").and_then(VoxelKind::from_key))
                    .unwrap_or(VoxelKind::Stone);
                let axis = if name.ends_with("_top") { 1 } else { 0 };
                [1.0, 1.0, 1.0, self.texture_layer(kind, axis, true) as f32]
            })
            .collect::<Vec<_>>();
        table.extend(self.face_table());
        // Tint palette, read from the end of the table by `model_vertex.wesl`.
        table.extend(super::models::TINTS.map(|[r, g, b]| [r, g, b, 0.0]));
        table
    }
    pub(crate) fn model_face_buffer(&self) -> ShaderBuffer {
        ShaderBuffer::from(self.model_face_table())
    }

    /// `(tint, layer | uv transform << 12)` of each greedy cube appearance, six
    /// faces each indexed by direction, as read by `voxel_material.wesl`.
    pub(crate) fn cube_face_table(&self) -> Vec<[f32; 4]> {
        let model = self.model_face_table();
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

    pub(crate) fn cube_face_buffer(&self) -> ShaderBuffer {
        ShaderBuffer::from(self.cube_face_table())
    }
}

pub(crate) fn block_texture(asset_server: &AssetServer, path: &str, layers: u32) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(move |settings: &mut ImageLoaderSettings| {
            settings.array_layout = Some(ImageArrayLayout::RowCount { rows: layers });
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Nearest,
                min_filter: ImageFilterMode::Nearest,
                // Blends between the levels of `block_mipmaps`, so no mip seams show.
                mipmap_filter: ImageFilterMode::Linear,
                ..default()
            });
        })
        .load(path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn built_in_packs_have_valid_layers_and_assets() {
        for id in TexturePackId::all() {
            let pack = TexturePack::load(id);
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets")
                    .join(&pack.texture_path)
                    .is_file()
            );
            for &kind in VoxelKind::itemizable() {
                for axis in 0..3 {
                    for positive in [false, true] {
                        assert!(pack.texture_layer(kind, axis, positive) < pack.layers);
                    }
                }
            }
        }
        assert!(TexturePackId::default().is_available());
    }
}
