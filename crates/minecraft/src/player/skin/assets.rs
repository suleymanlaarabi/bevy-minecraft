//! Cached meshes, materials, and normalized skin textures.
use super::*;
use bevy::{asset::LoadState, render::render_resource::TextureFormat};

pub(super) fn setup(
    mut assets: ResMut<SkinAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
) {
    assets.preview = images.add(Image::new_target_texture(
        272,
        408,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    assets.arm_view = images.add(Image::new_target_texture(
        1,
        1,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    assets.classic = mesh::skin_parts(&mut meshes);
    assets.source = crate::load_nearest(&server, "skin.png");
    assets.base = materials.add(StandardMaterial {
        perceptual_roughness: 1.,
        reflectance: 0.,
        ..default()
    });
    assets.outer = materials.add(StandardMaterial {
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 1.,
        reflectance: 0.,
        cull_mode: None,
        ..default()
    });
    assets.arm_base = materials.add(StandardMaterial {
        unlit: true,
        ..default()
    });
    assets.arm_outer = materials.add(StandardMaterial {
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    assets.armor_meshes = mesh::armor_parts(&mut meshes, 1.);
    assets.leggings_meshes = mesh::armor_parts(&mut meshes, 0.5);
    let mut armor = std::collections::HashMap::new();
    let root = crate::asset_directory();
    // Resolve names once and load only the shared materials referenced by actual equipment.
    assets.armor = (0..crate::inventory::ITEM_ID_LIMIT)
        .map(|id| {
            let item = crate::inventory::ItemKind(id);
            let Some(name) = pumpkin_data::item::Item::from_id(id)
                .and_then(|_| item.component::<pumpkin_data::data_component_impl::EquippableImpl>())
                .and_then(|equipment| equipment.asset_id.as_deref())
            else {
                return u8::MAX;
            };
            let name = name.strip_prefix("minecraft:").unwrap_or(name);
            let leggings = item.armor_slot() == Some(crate::inventory::ARMOR_START + 2);
            *armor.entry((name, leggings)).or_insert_with(|| {
                let layer = if leggings {
                    "humanoid_leggings"
                } else {
                    "humanoid"
                };
                let path = format!("player/vanilla/entity/equipment/{layer}/{name}.png");
                if !root.join(&path).is_file() {
                    return u8::MAX;
                }
                let index = u8::try_from(assets.armor_materials.len())
                    .ok()
                    .filter(|&index| index != u8::MAX)
                    .expect("Armor material index must fit below the missing-material sentinel");
                let material = materials.add(StandardMaterial {
                    base_color_texture: Some(crate::load_nearest(&server, path)),
                    alpha_mode: AlphaMode::Mask(0.5),
                    base_color: if name == "leather" {
                        Color::srgb_u8(160, 101, 64)
                    } else {
                        Color::WHITE
                    },
                    perceptual_roughness: 0.8,
                    reflectance: 0.1,
                    ..default()
                });
                assets.armor_materials.push(material);
                index
            })
        })
        .collect();
}

pub(super) fn load_skin(
    server: Res<AssetServer>,
    mut assets: ResMut<SkinAssets>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut events: MessageReader<AssetEvent<Image>>,
) {
    let modified = events
        .read()
        .any(|event| matches!(event, AssetEvent::Modified { id } if *id == assets.source.id()));
    if modified {
        assets.loaded = false;
    }
    if assets.loaded {
        return;
    }
    if matches!(
        server.get_load_state(assets.source.id()),
        Some(LoadState::Failed(_))
    ) {
        error!("Unable to load default skin.");
        assets.loaded = true;
        return;
    }
    let Some(source) = images.get(&assets.source) else {
        return;
    };
    match texture::normalize(source) {
        Ok(image) => {
            let handle = if assets.texture == Handle::default() {
                images.add(image)
            } else {
                let _ = images.insert(assets.texture.id(), image);
                assets.texture.clone()
            };
            assets.texture = handle.clone();
            for material in [
                &assets.base,
                &assets.outer,
                &assets.arm_base,
                &assets.arm_outer,
            ] {
                if let Some(mut material) = materials.get_mut(material) {
                    material.base_color_texture = Some(handle.clone());
                }
            }
        }
        Err(error) => {
            error!("Invalid default skin: {error}");
            assets.loaded = true;
            return;
        }
    }
    assets.loaded = true;
}
