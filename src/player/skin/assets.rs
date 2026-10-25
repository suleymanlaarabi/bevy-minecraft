//! Cached meshes, materials, and normalized skin textures.
use super::*;
use bevy::{
    asset::LoadState,
    image::{ImageLoaderSettings, ImageSampler},
    render::render_resource::TextureFormat,
};

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
    assets.classic = mesh::skin_parts(&mut meshes, false);
    assets.slim = mesh::skin_parts(&mut meshes, true);
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
    let root = crate::asset_directory().join("player/vanilla/entity/equipment");
    for layer in ["humanoid", "humanoid_leggings"] {
        let Ok(entries) = std::fs::read_dir(root.join(layer)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "png") {
                continue;
            }
            let name = path.file_stem().unwrap().to_string_lossy();
            let key = format!("{layer}/{name}");
            let image = crate::load_nearest(
                &server,
                format!("player/vanilla/entity/equipment/{key}.png"),
            );
            let material = materials.add(StandardMaterial {
                base_color_texture: Some(image),
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
            assets.armor.insert(key, material);
        }
    }
}

pub(super) fn load_skin(
    settings: Res<SkinSettings>,
    server: Res<AssetServer>,
    mut assets: ResMut<SkinAssets>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut events: MessageReader<AssetEvent<Image>>,
) {
    let modified = events
        .read()
        .any(|event| matches!(event, AssetEvent::Modified { id } if *id == assets.source.id()));
    if settings.texture != assets.requested {
        assets.requested = settings.texture.clone();
        assets.source = crate::load_nearest(&server, settings.texture.clone());
        assets.loaded = false;
        assets.fallback = false;
        assets.status = "Loading skin…".into();
    }
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
        skin_error(&mut assets, &server, "Unable to load skin.");
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
            if !assets.fallback {
                assets.status = IMPORT_HINT.into();
            }
        }
        Err(error) => {
            skin_error(&mut assets, &server, error);
            return;
        }
    }
    assets.loaded = true;
}

fn skin_error(assets: &mut SkinAssets, server: &AssetServer, error: &str) {
    assets.status = error.into();
    if assets.texture == Handle::default() && !assets.fallback && assets.requested != "skin.png" {
        assets.source = crate::load_nearest(server, "skin.png".into());
        assets.fallback = true;
        assets.status.push_str(" Using default skin.");
    } else {
        assets.loaded = true;
    }
}
