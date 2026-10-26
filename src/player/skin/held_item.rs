//! Cached first-person item visuals and hotbar equip transitions.
use super::*;
use crate::{
    inventory::{ItemKind, PlayerInventory, icons::BlockIcons},
    player::Player,
    voxel::{
        VoxelAssets,
        render::{ModelMaterial, model_mesh, models},
    },
};
use bevy::{
    asset::{AssetLoadFailedEvent, AssetLoader, LoadContext, RenderAssetUsages, io::Reader},
    camera::visibility::RenderLayers,
    light::{NotShadowCaster, NotShadowReceiver},
    render::render_resource::PrimitiveTopology,
};
use serde::Deserialize;
use std::f32::consts::PI;

#[derive(Component)]
pub(super) struct FirstPersonRoot;

#[derive(Component, Default)]
#[require(DesiredItem, EquipTransition)]
pub(super) struct HeldItem {
    pub item: Option<ItemKind>,
    pub equipped: f32,
    pub display: Transform,
    initialized: bool,
}

#[derive(Component)]
pub(super) struct HeldItemPart;

#[derive(Clone, Deserialize)]
#[serde(default)]
struct DisplayTransform {
    rotation: Vec3,
    translation: Vec3,
    scale: Vec3,
}

impl Default for DisplayTransform {
    fn default() -> Self {
        Self {
            rotation: Vec3::ZERO,
            translation: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

impl DisplayTransform {
    fn transform(&self) -> Transform {
        Transform {
            translation: self.translation / 16.,
            rotation: Quat::from_euler(
                EulerRot::XYZ,
                self.rotation.x.to_radians(),
                self.rotation.y.to_radians(),
                self.rotation.z.to_radians(),
            ),
            scale: self.scale,
        }
    }
}

#[derive(Deserialize)]
struct ItemDefinition {
    display: DisplayTransform,
    sprite: Option<String>,
}

#[derive(Clone)]
enum ItemMaterial {
    Block(Handle<ModelMaterial>),
    Sprite(Handle<StandardMaterial>),
}

#[derive(Clone)]
struct ItemVisual {
    mesh: Handle<Mesh>,
    material: ItemMaterial,
    local: Transform,
    display: Transform,
}

#[derive(Asset, TypePath, Deserialize)]
pub(super) struct HeldManifest {
    version: u32,
    #[serde(deserialize_with = "item_definitions")]
    items: HashMap<u16, ItemDefinition>,
}
fn item_definitions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<HashMap<u16, ItemDefinition>, D::Error> {
    let definitions = HashMap::<String, ItemDefinition>::deserialize(deserializer)?;
    let mut items = HashMap::with_capacity(definitions.len());
    for (name, definition) in definitions {
        let item = pumpkin_data::item::Item::from_registry_key(&name).ok_or_else(|| {
            serde::de::Error::custom(format!("Unknown held-item identity: {name}"))
        })?;
        if items.insert(item.id, definition).is_some() {
            return Err(serde::de::Error::custom(format!(
                "Duplicate held-item identity: {name}"
            )));
        }
    }
    Ok(items)
}
#[derive(Default, TypePath)]
pub(super) struct HeldManifestLoader;
impl AssetLoader for HeldManifestLoader {
    type Asset = HeldManifest;
    type Settings = ();
    type Error = std::io::Error;
    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let manifest: HeldManifest =
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        if manifest.version != 1 {
            return Err(std::io::Error::other(
                "Unsupported held-item manifest; reimport player assets.",
            ));
        }
        Ok(manifest)
    }
    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}
#[derive(Resource, Default)]
pub(super) struct HeldItemAssets {
    manifest: Handle<HeldManifest>,
    sources: HashMap<ItemKind, Handle<Image>>,
    visuals: HashMap<ItemKind, ItemVisual>,
    failed: bevy::platform::collections::HashSet<ItemKind>,
}
#[derive(Component, Default, PartialEq, Eq)]
pub(super) struct DesiredItem {
    item: Option<ItemKind>,
    rebuild: bool,
}
#[derive(Component, Default)]
pub(super) struct EquipTransition;
#[derive(Component)]
pub(super) struct BuildVisual;
#[derive(Component)]
pub(super) struct WaitingForVisual;

pub(super) fn setup(mut assets: ResMut<HeldItemAssets>, server: Res<AssetServer>) {
    assets.manifest = server.load("player/vanilla/held_items.json");
}
pub(super) fn select(
    mut commands: Commands,
    inventory: Single<(&PlayerInventory, &crate::inventory::HotbarSelection), With<Player>>,
    mut held: Query<(Entity, &mut DesiredItem)>,
) {
    let (inventory, selection) = *inventory;
    let selected = selection.stack(inventory).map(|stack| stack.item());
    for (entity, mut desired) in &mut held {
        if desired.item != selected {
            desired.item = selected;
            desired.rebuild = true;
            commands
                .entity(entity)
                .remove::<WaitingForVisual>()
                .insert(EquipTransition);
        }
    }
}
pub(super) fn advance_equipment(
    mut commands: Commands,
    time: Res<Time>,
    mut held: Query<(Entity, &DesiredItem, &mut HeldItem), With<EquipTransition>>,
) {
    for (entity, desired, mut held) in &mut held {
        let changing = desired.rebuild || !held.initialized || desired.item != held.item;
        held.equipped = held
            .equipped
            .move_towards(if changing { 0.0 } else { 1.0 }, time.delta_secs() * 8.0);
        if changing && held.equipped <= 0.1 {
            commands
                .entity(entity)
                .remove::<EquipTransition>()
                .insert(BuildVisual);
        } else if !changing && held.equipped == 1.0 {
            commands.entity(entity).remove::<EquipTransition>();
        }
    }
}
pub(super) fn asset_events(
    mut commands: Commands,
    mut images: MessageReader<AssetEvent<Image>>,
    mut manifests: MessageReader<AssetEvent<HeldManifest>>,
    mut failures: MessageReader<AssetLoadFailedEvent<Image>>,
    mut manifest_failures: MessageReader<AssetLoadFailedEvent<HeldManifest>>,
    mut assets: ResMut<HeldItemAssets>,
    mut held: Query<(Entity, &mut DesiredItem, Has<WaitingForVisual>)>,
) {
    let manifest_changed = manifests.read().any(|event| matches!(event, AssetEvent::Added { id } | AssetEvent::Modified { id } if *id == assets.manifest.id()));
    let manifest_failed = manifest_failures
        .read()
        .any(|event| event.id == assets.manifest.id());
    let changed: Vec<_> = images
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    let failed: Vec<_> = failures.read().map(|event| event.id).collect();
    if manifest_changed {
        assets.visuals.clear();
        assets.sources.clear();
        assets.failed.clear();
    }
    for (entity, mut desired, waiting) in &mut held {
        let source = desired
            .item
            .and_then(|item| assets.sources.get(&item))
            .map(Handle::id);
        let reloaded = source.is_some_and(|source| changed.contains(&source));
        if manifest_changed || reloaded {
            if let Some(item) = desired.item {
                assets.visuals.remove(&item);
                assets.failed.remove(&item);
            }
            desired.rebuild = true;
            commands
                .entity(entity)
                .remove::<WaitingForVisual>()
                .insert(EquipTransition);
        } else if waiting
            && (manifest_failed
                || !changed.is_empty()
                || source.is_some_and(|source| failed.contains(&source)))
        {
            commands
                .entity(entity)
                .remove::<WaitingForVisual>()
                .insert(BuildVisual);
        }
    }
}
pub(super) fn build_visual(
    mut commands: Commands,
    mut held: Query<(Entity, &mut DesiredItem, &mut HeldItem), With<BuildVisual>>,
    parts: Query<(Entity, &ChildOf), With<HeldItemPart>>,
    mut assets: ResMut<HeldItemAssets>,
    manifests: Res<Assets<HeldManifest>>,
    server: Res<AssetServer>,
    images: Res<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    voxel_assets: Res<VoxelAssets>,
    icons: Res<BlockIcons>,
    sky: Res<crate::sky::SkyState>,
) {
    for (root, mut desired, mut held) in &mut held {
        commands.entity(root).remove::<BuildVisual>();
        let visual = if let Some(item) = desired.item {
            if !matches!(
                server.get_load_state(assets.manifest.id()),
                Some(bevy::asset::LoadState::Loaded | bevy::asset::LoadState::Failed(_))
            ) {
                commands.entity(root).insert(WaitingForVisual);
                continue;
            }
            let definition = manifests
                .get(&assets.manifest)
                .and_then(|manifest| manifest.items.get(&item.registry().id));
            let Some(visual) = assets.visual(
                item,
                definition,
                &server,
                &images,
                &mut meshes,
                &mut materials,
                &voxel_assets,
                &icons,
                &sky,
            ) else {
                commands.entity(root).insert(WaitingForVisual);
                continue;
            };
            Some(visual)
        } else {
            None
        };
        for (entity, parent) in &parts {
            if parent.parent() == root {
                commands.entity(entity).despawn();
            }
        }
        held.item = desired.item;
        desired.rebuild = false;
        if let Some(visual) = visual {
            held.display = visual.display;
            let mut entity = commands.spawn((
                HeldItemPart,
                Mesh3d(visual.mesh),
                visual.local,
                ChildOf(root),
                RenderLayers::layer(ARM_LAYER),
                NotShadowCaster,
                NotShadowReceiver,
            ));
            match visual.material {
                ItemMaterial::Block(material) => {
                    entity.insert(MeshMaterial3d(material));
                }
                ItemMaterial::Sprite(material) => {
                    entity.insert(MeshMaterial3d(material));
                }
            }
        }
        if !held.initialized {
            held.equipped = 1.0;
            held.initialized = true;
        }
        commands
            .entity(root)
            .remove::<WaitingForVisual>()
            .insert(EquipTransition);
    }
}

impl HeldItemAssets {
    #[expect(clippy::too_many_arguments)]
    fn visual(
        &mut self,
        item: ItemKind,
        definition: Option<&ItemDefinition>,
        server: &AssetServer,
        images: &Assets<Image>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        voxel_assets: &VoxelAssets,
        icons: &BlockIcons,
        sky: &crate::sky::SkyState,
    ) -> Option<ItemVisual> {
        if let Some(visual) = self.visuals.get(&item) {
            return Some(visual.clone());
        }
        let display = definition.map(|definition| definition.display.transform());
        let sprite = definition.and_then(|definition| definition.sprite.as_deref());
        let visual = if let ItemKind::Block(kind) = item
            && sprite.is_none()
        {
            let mut translucent = false;
            models::visit_quads(kind.default_state(), IVec3::ZERO, |quad| {
                translucent |= quad.translucent
            });
            ItemVisual {
                mesh: meshes.add(model_mesh::item_geometry(kind).into_mesh()),
                material: ItemMaterial::Block(if translucent {
                    voxel_assets.translucent.clone()
                } else {
                    voxel_assets.models_cutout.clone()
                }),
                local: Transform::from_translation(Vec3::splat(-0.5)),
                display: display.unwrap_or(Transform {
                    rotation: Quat::from_rotation_y(45_f32.to_radians()),
                    scale: Vec3::splat(0.4),
                    ..default()
                }),
            }
        } else {
            let source = self
                .sources
                .entry(item)
                .or_insert_with(|| {
                    sprite.map_or_else(
                        || icons.item_image(item).cloned().unwrap_or_default(),
                        |sprite| {
                            crate::load_nearest(server, format!("player/vanilla/held/{sprite}.png"))
                        },
                    )
                })
                .clone();
            let failed = matches!(
                server.get_load_state(source.id()),
                Some(bevy::asset::LoadState::Failed(_))
            );
            if failed && self.failed.insert(item) {
                error!(
                    "Held-item texture failed for {}; using the missing-item visual",
                    item.name()
                );
            }
            let mut source = if self.failed.contains(&item) {
                icons.missing_image().clone()
            } else {
                source
            };
            let Some(image) = images.get(&source) else {
                return None;
            };
            let (offset, scale) = if sprite.is_some() || self.failed.contains(&item) {
                (Vec2::ZERO, Vec2::ONE)
            } else {
                icons.item_uv(item).unwrap_or((Vec2::ZERO, Vec2::ONE))
            };
            let mesh = if let Some(mesh) = sprite_mesh(image, offset, scale) {
                mesh
            } else {
                if self.failed.insert(item) {
                    error!(
                        "Invalid held-item image for {}; using the missing-item visual",
                        item.name()
                    );
                }
                source = icons.missing_image().clone();
                let Some(mesh) = images
                    .get(&source)
                    .and_then(|image| sprite_mesh(image, Vec2::ZERO, Vec2::ONE))
                else {
                    return None;
                };
                mesh
            };
            let level = ARM_NIGHT_LEVEL + (1. - ARM_NIGHT_LEVEL) * (1. - sky.night);
            ItemVisual {
                mesh: meshes.add(mesh),
                material: ItemMaterial::Sprite(materials.add(StandardMaterial {
                    base_color_texture: Some(source),
                    base_color: Color::linear_rgb(level, level, level),
                    alpha_mode: AlphaMode::Mask(0.5),
                    unlit: true,
                    ..default()
                })),
                local: Transform::default(),
                display: display.unwrap_or(
                    DisplayTransform {
                        rotation: Vec3::new(0., -90., 25.),
                        translation: Vec3::new(1.13, 3.2, 1.13),
                        scale: Vec3::splat(0.68),
                    }
                    .transform(),
                ),
            }
        };
        self.visuals.insert(item, visual.clone());
        Some(visual)
    }

    pub(super) fn dim(&self, materials: &mut Assets<StandardMaterial>, color: Color) {
        for visual in self.visuals.values() {
            if let ItemMaterial::Sprite(handle) = &visual.material
                && materials
                    .get(handle)
                    .is_some_and(|material| material.base_color != color)
                && let Some(mut material) = materials.get_mut(handle)
            {
                material.base_color = color;
            }
        }
    }
}

pub(super) fn pose(progress: f32, equipped: f32, display: &Transform) -> Transform {
    let strike = (progress.sqrt() * PI).sin();
    let twist = (progress * progress * PI).sin();
    let motion = Mat4::from_translation(Vec3::new(
        0.56 - 0.4 * strike,
        -0.52 - 0.6 * (1. - equipped) + 0.2 * (progress.sqrt() * PI * 2.).sin(),
        -0.72 - 0.2 * (progress * PI).sin(),
    )) * Mat4::from_rotation_y((45. - 20. * twist).to_radians())
        * Mat4::from_rotation_z((-20. * strike).to_radians())
        * Mat4::from_rotation_x((-80. * strike).to_radians())
        * Mat4::from_rotation_y(-45_f32.to_radians());
    Transform::from_matrix(motion * display.to_matrix())
}

/// Front/back faces and silhouette edges; never extrude transparent pixels.
fn sprite_mesh(image: &Image, offset: Vec2, scale: Vec2) -> Option<Mesh> {
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
    let mut quad = |mut points: [Vec3; 4], normal: Vec3, mut uv: [Vec2; 4]| {
        if (points[1] - points[0])
            .cross(points[2] - points[0])
            .dot(normal)
            < 0.
        {
            points.reverse();
            uv.reverse();
        }
        let first = positions.len() as u32;
        positions.extend(points.map(|point| point.to_array()));
        normals.extend([normal.to_array(); 4]);
        uvs.extend(uv.map(|uv| (offset + uv * scale).to_array()));
        let shade = 1. - 0.2 * normal.x.abs() - 0.15 * normal.y.abs();
        colors.extend([[shade, shade, shade, 1.]; 4]);
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    };
    let depth = 1. / 32.;
    for (z, normal) in [(depth, Vec3::Z), (-depth, Vec3::NEG_Z)] {
        quad(
            [
                Vec3::new(-0.5, -0.5, z),
                Vec3::new(0.5, -0.5, z),
                Vec3::new(0.5, 0.5, z),
                Vec3::new(-0.5, 0.5, z),
            ],
            normal,
            [Vec2::new(0., 1.), Vec2::ONE, Vec2::X, Vec2::ZERO],
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
            for (neighbor, normal, a, b) in [
                (
                    (x - 1, y),
                    Vec3::NEG_X,
                    Vec2::new(left, bottom),
                    Vec2::new(left, top),
                ),
                (
                    (x + 1, y),
                    Vec3::X,
                    Vec2::new(right, bottom),
                    Vec2::new(right, top),
                ),
                (
                    (x, y - 1),
                    Vec3::Y,
                    Vec2::new(left, top),
                    Vec2::new(right, top),
                ),
                (
                    (x, y + 1),
                    Vec3::NEG_Y,
                    Vec2::new(left, bottom),
                    Vec2::new(right, bottom),
                ),
            ] {
                if !solid(neighbor.0, neighbor.1) {
                    quad(
                        [
                            a.extend(-depth),
                            b.extend(-depth),
                            b.extend(depth),
                            a.extend(depth),
                        ],
                        normal,
                        uv,
                    );
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
    .with_inserted_indices(model_mesh::compact_indices(indices, vertex_count));
    mesh.final_aabb = Some(bevy::shape::Aabb3d::from_min_max(
        Vec3::new(-0.5, -0.5, -depth),
        Vec3::new(0.5, 0.5, depth),
    ));
    Some(mesh)
}
