//! Inventory models are rasterized once per texture revision, off the ECS thread.
use crate::{
    inventory::ItemKind,
    voxel::{MinecraftTextures, VoxelAssets, VoxelKind, VoxelMaterial, render::models},
};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

const ICON_SIZE: usize = 64;
const ICON_COLUMNS: usize = 32;
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
    special_source: Option<AssetId<Image>>,
    special_failed: bool,
    items: Box<[Option<usize>]>,
    item_image: Handle<Image>,
    item_layout: Handle<TextureAtlasLayout>,
    item_grid: UVec2,
    missing: Handle<Image>,
    revision: u64,
    finished: Option<u64>,
    task: Option<Task<IconOutput>>,
}
impl BlockIcons {
    pub(crate) fn missing_image(&self) -> &Handle<Image> {
        &self.missing
    }

    fn item_index(&self, item: ItemKind) -> Option<usize> {
        self.items.get(usize::from(item.id())).copied().flatten()
    }

    pub(crate) fn item_image(&self, item: ItemKind) -> Option<&Handle<Image>> {
        item.block_kind().is_none().then(|| {
            self.item_index(item)
                .map_or(&self.missing, |_| &self.item_image)
        })
    }

    pub(crate) fn item_atlas(&self, item: ItemKind) -> Option<TextureAtlas> {
        Some(TextureAtlas {
            layout: self.item_layout.clone(),
            index: self.item_index(item)?,
        })
    }
    pub(crate) fn item_uv(&self, item: ItemKind) -> Option<(Vec2, Vec2)> {
        let index = self.item_index(item)? as u32;
        let scale = Vec2::ONE / self.item_grid.as_vec2();
        Some((
            UVec2::new(index % self.item_grid.x, index / self.item_grid.x).as_vec2() * scale,
            scale,
        ))
    }
    pub(crate) fn image(&self) -> &Handle<Image> {
        &self.image
    }
    pub(crate) fn atlas(&self, item: ItemKind) -> TextureAtlas {
        let Some(kind) = item.block_kind() else {
            return TextureAtlas {
                layout: self.layout.clone(),
                index: 0,
            };
        };
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
    server: Res<AssetServer>,
) {
    #[derive(serde::Deserialize)]
    struct ItemAtlasManifest {
        columns: u32,
        rows: u32,
        tile: u32,
        items: std::collections::HashMap<String, usize>,
    }
    let manifest = crate::asset_directory().join("player/vanilla/items.json");
    if let Ok(bytes) = std::fs::read(manifest)
        && let Ok(manifest) = serde_json::from_slice::<ItemAtlasManifest>(&bytes)
        && manifest.columns > 0
        && manifest.rows > 0
        && manifest.tile > 0
    {
        icons.item_grid = UVec2::new(manifest.columns, manifest.rows);
        icons.item_image = crate::load_nearest(&server, "player/vanilla/items.png");
        icons.item_layout = layouts.add(TextureAtlasLayout::from_grid(
            UVec2::splat(manifest.tile),
            manifest.columns,
            manifest.rows,
            None,
            None,
        ));
        icons.items = vec![None; super::model::ITEM_ID_LIMIT as usize].into_boxed_slice();
        for (name, index) in manifest.items {
            if let Some(item) = pumpkin_data::item::Item::from_registry_key(&name) {
                icons.items[item.id as usize] = Some(index);
            }
        }
    }
    // An explicit missing-item symbol also covers installations without imported sprites.
    let mut missing = vec![0; 16 * 16 * 4];
    let symbol = [
        0b01110_u8, 0b10001, 0b00001, 0b00110, 0b00100, 0b00000, 0b00100,
    ];
    for (y, row) in symbol.into_iter().enumerate() {
        for x in 0..5 {
            if row & (1 << (4 - x)) != 0 {
                let offset = ((y + 4) * 16 + x + 5) * 4;
                missing[offset..offset + 4].copy_from_slice(&[195, 195, 205, 255]);
            }
        }
    }
    let mut fallback = Image::new(
        Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        missing,
        TextureFormat::Rgba8UnormSrgb,
        // Held-item extrusion also reads this small placeholder on the CPU.
        RenderAssetUsages::default(),
    );
    fallback.sampler = bevy::image::ImageSampler::nearest();
    icons.missing = images.add(fallback);
    let rows = icon_count().div_ceil(ICON_COLUMNS);
    let size = Extent3d {
        width: (ICON_SIZE * ICON_COLUMNS) as u32,
        height: (ICON_SIZE * rows) as u32,
        depth_or_array_layers: 1,
    };
    icons.image = images.add(Image::new_uninit(
        size,
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        // Bevy keeps the descriptor and handle; later renders replace the pixels.
        RenderAssetUsages::RENDER_WORLD,
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
    server: Res<AssetServer>,
    voxel_assets: Res<VoxelAssets>,
    materials: Res<Assets<VoxelMaterial>>,
    model_materials: Res<Assets<bevy_voxel::render::material::ModelMaterial>>,
    textures: Res<MinecraftTextures>,
    render_source: Res<crate::voxel::render::MinecraftRenderSource>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(source) = materials.get(&voxel_assets.terrain).map(|m| m.blocks.id()) else {
        return;
    };
    let special_source = model_materials
        .get(&voxel_assets.models)
        .and_then(|material| material.special.as_ref())
        .map(Handle::id);
    let modified = events
        .read()
        .any(|e| matches!(e,AssetEvent::Added{id} | AssetEvent::Modified{id} if *id==source || Some(*id)==special_source));
    if icons.source != Some(source) || icons.special_source != special_source || modified {
        icons.source = Some(source);
        icons.special_source = special_source;
        icons.special_failed = false;
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
    if !icons.special_failed
        && let Some(source) = special_source
        && let Some(bevy::asset::LoadState::Failed(error)) = server.get_load_state(source)
    {
        warn!("Special icon textures failed to load; using available block textures: {error}");
        icons.special_failed = true;
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
    // Render immediately with the available textures. An Added/Modified event
    // regenerates the atlas when optional special textures become available.
    let special = special_source
        .and_then(|source| images.get(source))
        .filter(|image| image.data.is_some())
        .cloned();
    let blocks = blocks.clone();
    let table = textures.model_face_table();
    let revision = icons.revision;
    let render_source = render_source.clone();
    icons.task = Some(AsyncComputeTaskPool::get().spawn(async move {
        IconOutput {
            revision,
            pixels: bevy_voxel::render::icons::rasterize_icons(
                &render_source,
                VoxelKind::itemizable().iter().map(|kind| {
                    (
                        bevy_voxel::core::StateId(kind.default_state().id()),
                        Vec3::from_array(models::tint(*kind)),
                    )
                }),
                &blocks,
                special.as_ref(),
                &table,
                ICON_SIZE,
                ICON_COLUMNS,
            ),
        }
    }));
}
