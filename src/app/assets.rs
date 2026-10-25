/// Directory the asset server reads from.
pub fn asset_directory() -> std::path::PathBuf {
    if let Some(root) = std::env::var_os("BEVY_ASSET_ROOT") {
        return std::path::PathBuf::from(root).join("assets");
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let assets = parent.join("assets");
        if assets.is_dir() {
            return assets;
        }
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

/// Loads a pixel-art image with nearest-neighbor sampling.
pub fn load_nearest(
    server: &bevy::asset::AssetServer,
    path: impl Into<bevy::asset::AssetPath<'static>>,
) -> bevy::asset::Handle<bevy::image::Image> {
    server
        .load_builder()
        .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
            settings.sampler = bevy::image::ImageSampler::nearest();
        })
        .load(path)
}
