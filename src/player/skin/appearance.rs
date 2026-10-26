//! Persistent appearance controls and asynchronous PNG import.
use super::*;
use bevy::{
    image::{CompressedImageFormats, ImageSampler, ImageType},
    settings::SaveSettingsDeferred,
    tasks::{AsyncComputeTaskPool, futures::check_ready},
    window::FileDragAndDrop,
};

pub(super) fn import_skin(
    mut events: MessageReader<FileDragAndDrop>,
    state: Option<Res<State<InventoryState>>>,
    sessions: Query<(), With<crate::block_behavior::containers::ViewingContainer>>,
    mut assets: ResMut<SkinAssets>,
    mut import: ResMut<SkinImport>,
) {
    for event in events.read() {
        let FileDragAndDrop::DroppedFile { path_buf, .. } = event else {
            continue;
        };
        if !sessions.is_empty()
            || state
                .as_ref()
                .is_none_or(|state| *state.get() != InventoryState::Open)
        {
            continue;
        }
        if path_buf
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("png"))
        {
            assets.status = "Choose a PNG skin.".into();
            continue;
        }
        let path = path_buf.clone();
        let asset_root = crate::asset_directory();
        assets.status = "Importing skin…".into();
        import.0 = Some(AsyncComputeTaskPool::get().spawn(async move {
            (|| -> Result<String, String> {
                if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
                    return Err("Skin file exceeds 8 MiB.".into());
                }
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") || bytes.len() < 33 {
                    return Err("Invalid PNG skin.".into());
                }
                let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                if !(64..=2048).contains(&width)
                    || width % 64 != 0
                    || ![width, width / 2].contains(&height)
                {
                    return Err(
                        "Use a 64x64 or 64x32 skin, or a scaled version up to 2048 pixels.".into(),
                    );
                }
                let image = Image::from_buffer(
                    &bytes,
                    ImageType::Extension("png"),
                    CompressedImageFormats::NONE,
                    true,
                    ImageSampler::nearest(),
                    bevy::asset::RenderAssetUsages::default(),
                )
                .map_err(|e| e.to_string())?;
                texture::normalize(&image).map_err(str::to_owned)?;
                let path = format!("player/custom/{:08x}.png", crc32fast::hash(&bytes));
                let target = asset_root.join(&path);
                std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::write(target, bytes).map_err(|e| e.to_string())?;
                Ok(path)
            })()
        }));
    }
}

pub(super) fn finish_import(
    mut import: ResMut<SkinImport>,
    settings: Res<SkinSettings>,
    mut assets: ResMut<SkinAssets>,
    mut commands: Commands,
) {
    // Polling must not flag `SkinImport` as changed every frame.
    let Some(task) = import.bypass_change_detection().0.as_mut() else {
        return;
    };
    let Some(result) = check_ready(task) else {
        return;
    };
    import.0 = None;
    match result {
        Ok(texture) => {
            save(
                &mut commands,
                SkinSettings {
                    texture,
                    ..settings.clone()
                },
            );
            assets.status = IMPORT_HINT.into();
        }
        Err(error) => assets.status = format!("Skin import failed: {error}"),
    }
}

pub(crate) const IMPORT_HINT: &str = "Drop a skin PNG to import";

fn save(commands: &mut Commands, settings: SkinSettings) {
    commands.insert_resource(settings);
    commands.queue(SaveSettingsDeferred(crate::settings::SAVE_DELAY));
}

pub(crate) fn reset_skin(
    mut event: On<PointerClick>,
    mut import: ResMut<SkinImport>,
    mut assets: ResMut<SkinAssets>,
    mut commands: Commands,
) {
    event.propagate(false);
    if event.button != PointerButton::Primary {
        return;
    }
    import.0 = None;
    assets.status = IMPORT_HINT.into();
    save(&mut commands, SkinSettings::default());
}

pub(crate) fn toggle_model(
    mut event: On<PointerClick>,
    settings: Res<SkinSettings>,
    mut commands: Commands,
) {
    event.propagate(false);
    if event.button != PointerButton::Primary {
        return;
    }
    save(
        &mut commands,
        SkinSettings {
            model: if settings.model == SkinModel::Classic {
                SkinModel::Slim
            } else {
                SkinModel::Classic
            },
            ..settings.clone()
        },
    );
}
pub(crate) fn toggle_layers(
    mut event: On<PointerClick>,
    settings: Res<SkinSettings>,
    mut commands: Commands,
) {
    event.propagate(false);
    if event.button != PointerButton::Primary {
        return;
    }
    save(
        &mut commands,
        SkinSettings {
            outer_layer: !settings.outer_layer,
            ..settings.clone()
        },
    );
}

pub(super) fn sync_labels(
    settings: Res<SkinSettings>,
    assets: Res<SkinAssets>,
    mut labels: Query<
        (
            &mut Text,
            Has<SkinModelLabel>,
            Has<SkinLayersLabel>,
            Has<SkinStatusLabel>,
        ),
        Or<(
            With<SkinModelLabel>,
            With<SkinLayersLabel>,
            With<SkinStatusLabel>,
        )>,
    >,
) {
    for (mut text, model, layers, status) in &mut labels {
        let value = if model {
            match settings.model {
                SkinModel::Classic => "Classic arms",
                SkinModel::Slim => "Slim arms",
            }
        } else if layers {
            if settings.outer_layer {
                "Layers: on"
            } else {
                "Layers: off"
            }
        } else if status {
            &assets.status
        } else {
            continue;
        };
        if text.0 != value {
            text.0 = value.into();
        }
    }
}
