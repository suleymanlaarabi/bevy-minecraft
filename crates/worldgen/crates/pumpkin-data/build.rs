use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MC_VERSION: &str = "26.3";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or("Failed to resolve repository root directory")?;
    let assets_dir = repo_root.join("assets");
    let datapack_dir = assets_dir.join("datapack");
    let mc_data_dir = datapack_dir.join("data").join("minecraft");

    println!("cargo:rerun-if-changed={}", mc_data_dir.display());
    // A missing rerun-if-changed path makes Cargo rerun this script on every build.
    let conventional_tags = assets_dir.join("conventional_tags");
    if conventional_tags.is_dir() {
        println!("cargo:rerun-if-changed={}", conventional_tags.display());
    }

    if mc_data_dir.is_dir() {
        copy_conventional_tags(&assets_dir, &datapack_dir);
        return Ok(());
    }

    Err(format!(
        "Vanilla datapack missing at {}. Place Minecraft {MC_VERSION} data there.",
        mc_data_dir.display()
    )
    .into())
}

fn copy_conventional_tags(assets_dir: &Path, datapack_dir: &Path) {
    let src = assets_dir.join("conventional_tags").join("data").join("c");
    let dst = datapack_dir.join("data").join("c");
    if src.is_dir() && !dst.is_dir() {
        let _ = copy_dir_all(&src, &dst);
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
