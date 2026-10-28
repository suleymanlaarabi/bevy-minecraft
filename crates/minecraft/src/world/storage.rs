//! Blocking transactional storage. Call only from workers, never ECS systems.
use super::model::{Checkpoint, WorldMetadata};
use hollow_worldgen::ChunkStorage;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use hollow_engine::storage::sqlite::SqliteStore;
pub(super) use hollow_engine::storage::sqlite::{EncodedColumn, PreparedCheckpoint};
fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

pub(super) fn root() -> io::Result<PathBuf> {
    #[cfg(feature = "dev")]
    if std::env::var_os("HOLLOW_FLIGHT_BENCH").is_some() {
        return Ok(std::env::temp_dir().join("hollow-flight-worlds"));
    }
    if let Some(path) = std::env::var_os("HOLLOW_WORLDS_DIR") {
        return Ok(path.into());
    }
    directories::ProjectDirs::from("org", "hollow", "game")
        .map(|p| p.data_dir().join("worlds"))
        .ok_or_else(|| err("User data directory is unavailable."))
}

#[derive(Clone)]
pub(super) struct CatalogEntry {
    pub path: PathBuf,
    pub metadata: Option<WorldMetadata>,
    pub error: Option<String>,
}

pub(super) fn catalog() -> io::Result<Vec<CatalogEntry>> {
    let root = root()?;
    std::fs::create_dir_all(&root)?;
    let mut worlds = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(DELETING_PREFIX))
        {
            // Finish a deletion an earlier run was interrupted in.
            let _ = std::fs::remove_dir_all(entry.path());
            continue;
        }
        let path = entry.path().join("world.sqlite3");
        if !path.exists() {
            continue;
        }
        let metadata = (|| -> io::Result<WorldMetadata> {
            let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(err)?;
            read_json(&conn, "metadata")?.ok_or_else(|| err("Missing world metadata."))
        })();
        let (metadata, error) = match metadata {
            Ok(m) => {
                let error = m.validate().err().map(|e| e.to_string());
                (Some(m), error)
            }
            Err(e) => (None, Some(e.to_string())),
        };
        worlds.push(CatalogEntry {
            path,
            metadata,
            error,
        });
    }
    worlds.sort_by_key(|entry| {
        std::cmp::Reverse(entry.metadata.as_ref().map_or(0, |m| m.last_played))
    });
    Ok(worlds)
}

pub(super) fn delete_world(path: &Path) -> io::Result<()> {
    let folder = path.parent().ok_or_else(|| err("Invalid world path."))?;
    // Only catalogued world folders directly inside the configured root may be removed.
    if path.file_name().is_none_or(|name| name != "world.sqlite3")
        || std::fs::symlink_metadata(folder)?.file_type().is_symlink()
    {
        return Err(err("Invalid world path."));
    }
    let folder = folder.canonicalize()?;
    if folder.parent() != Some(root()?.canonicalize()?.as_path()) {
        return Err(err("The world is outside the worlds directory."));
    }
    let name = folder
        .file_name()
        .ok_or_else(|| err("Invalid world path."))?;
    let mut tombstone = std::ffi::OsString::from(DELETING_PREFIX);
    tombstone.push(name);
    let tombstone = folder.with_file_name(tombstone);
    {
        // Coordinate with open sessions in this process and other game instances.
        // Renaming under the lock detaches the world atomically: a waiter that
        // acquires the lock afterwards finds no lock file at the world's path.
        let _lock = SqliteStore::lock(&folder)?;
        std::fs::rename(&folder, &tombstone)?;
    }
    std::fs::remove_dir_all(tombstone)
}

/// Prefix of world folders detached by [`delete_world`] and not yet removed.
const DELETING_PREFIX: &str = ".deleting-";

pub(crate) struct Database(SqliteStore);
impl std::ops::Deref for Database {
    type Target = SqliteStore;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Database {
    pub(super) fn create(metadata: &WorldMetadata) -> io::Result<Arc<Self>> {
        let root = root()?;
        std::fs::create_dir_all(&root)?;
        let folder = root.join(&metadata.id);
        std::fs::create_dir(&folder)?;
        let result = SqliteStore::create(
            &folder.join("world.sqlite3"),
            &serde_json::to_vec(metadata).map_err(err)?,
        )
        .map(|store| Arc::new(Self(store)));
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&folder);
        }
        result
    }

    pub(super) fn open(path: &Path) -> io::Result<(Arc<Self>, WorldMetadata, Checkpoint)> {
        let store = SqliteStore::open(path)?;
        let metadata: WorldMetadata =
            decode_record(&store, "metadata")?.ok_or_else(|| err("Missing world metadata."))?;
        metadata.validate()?;
        let checkpoint: Checkpoint = decode_record(&store, "checkpoint")?.unwrap_or_default();
        if let Some(player) = &checkpoint.player {
            player.inventory()?;
        }
        if checkpoint.pending.iter().any(|(p, s)| {
            !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&p[1])
                || crate::voxel::VoxelState::from_id(*s).is_none()
        }) {
            return Err(err("Invalid pending block edits."));
        }
        Ok((Arc::new(Self(store)), metadata, checkpoint))
    }
}
impl ChunkStorage for Database {
    fn read_chunks(
        &self,
        positions: &[(i32, i32)],
    ) -> io::Result<Vec<Option<hollow_worldgen::codec::GenerationRecord>>> {
        self.0
            .read_provider(positions)?
            .into_iter()
            .map(|record| {
                record
                    .map(|(stage, bytes)| {
                        if !(1..hollow_worldgen::chunk_system::StagedChunkEnum::COUNT as u8)
                            .contains(&stage)
                        {
                            return Err(err("Invalid stored generation stage."));
                        }
                        Ok(hollow_worldgen::codec::GenerationRecord {
                            stage: hollow_worldgen::chunk_system::StagedChunkEnum::from(stage),
                            bytes,
                        })
                    })
                    .transpose()
            })
            .collect()
    }
    fn write_chunks(
        &self,
        chunks: Vec<((i32, i32), hollow_worldgen::codec::GenerationRecord)>,
    ) -> io::Result<()> {
        let records: Vec<_> = chunks
            .into_iter()
            .map(|(p, record)| (p, record.stage as u8, record.bytes))
            .collect();
        self.0.write_provider(records)
    }
    fn flush(&self) -> io::Result<()> {
        self.0.flush()
    }
}
fn read_json<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    key: &str,
) -> io::Result<Option<T>> {
    let bytes: Option<Vec<u8>> = conn
        .query_row("SELECT value FROM records WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    bytes
        .map(|b| {
            serde_json::from_slice(&b)
                .map_err(|error| err(format!("Invalid {key} record: {error}")))
        })
        .transpose()
}

fn decode_record<T: serde::de::DeserializeOwned>(
    store: &SqliteStore,
    key: &str,
) -> io::Result<Option<T>> {
    store
        .record(key)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(err))
        .transpose()
}
