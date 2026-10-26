//! Blocking transactional storage. Call only from workers, never ECS systems.
use super::model::{Checkpoint, WorldMetadata};
use hollow_worldgen::ChunkStorage;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError, RwLock},
    time::Duration,
};

use hollow_worldgen::codec::{
    MAX_RECORD_BYTES, compress_record as compress, decompress_record as decompress,
};
const SCHEMA: &str = "
    CREATE TABLE records(key TEXT PRIMARY KEY, value BLOB NOT NULL);
    CREATE TABLE generated(x INTEGER NOT NULL,z INTEGER NOT NULL,stage INTEGER NOT NULL,value BLOB NOT NULL,PRIMARY KEY(x,z)) WITHOUT ROWID;
    CREATE TABLE containers(x INTEGER NOT NULL,z INTEGER NOT NULL,value BLOB NOT NULL,PRIMARY KEY(x,z)) WITHOUT ROWID;
    CREATE TABLE edited(x INTEGER NOT NULL,z INTEGER NOT NULL,value BLOB NOT NULL,PRIMARY KEY(x,z)) WITHOUT ROWID;
";

fn err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

pub(super) fn root() -> io::Result<PathBuf> {
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
        let _lock = Database::lock(&folder)?;
        std::fs::rename(&folder, &tombstone)?;
    }
    std::fs::remove_dir_all(tombstone)
}

/// Prefix of world folders detached by [`delete_world`] and not yet removed.
const DELETING_PREFIX: &str = ".deleting-";

/// Presence and a process-local revision; odd revisions denote an in-flight commit.
#[derive(Clone, Copy, Default)]
struct ColumnState {
    terrain: bool,
    containers: bool,
    revision: u64,
}
#[derive(Default)]
struct ColumnUpdate {
    terrain: bool,
    containers: Option<bool>,
}

/// Compressed WAL data travels to a compute worker before it is decoded.
#[derive(Default)]
pub(crate) struct GameplayColumn {
    pub revision: u64,
    terrain: Option<Vec<u8>>,
    containers: Option<Vec<u8>>,
}
impl GameplayColumn {
    pub fn decode(self) -> io::Result<(Option<Vec<u8>>, Option<Vec<u8>>)> {
        Ok((
            self.terrain.map(|bytes| decompress(&bytes)).transpose()?,
            self.containers
                .map(|bytes| decompress(&bytes))
                .transpose()?,
        ))
    }
}

/// Publishes a stable revision on success or rollback, without holding a lock over SQL.
struct ColumnCommit<'a> {
    columns: &'a RwLock<HashMap<(i32, i32), ColumnState>>,
    updates: HashMap<(i32, i32), ColumnUpdate>,
    committed: bool,
}
impl<'a> ColumnCommit<'a> {
    fn begin(
        columns: &'a RwLock<HashMap<(i32, i32), ColumnState>>,
        updates: HashMap<(i32, i32), ColumnUpdate>,
    ) -> Self {
        {
            let mut states = columns.write().unwrap_or_else(PoisonError::into_inner);
            for &position in updates.keys() {
                states.entry(position).or_default().revision += 1;
            }
        }
        Self {
            columns,
            updates,
            committed: false,
        }
    }
}
impl Drop for ColumnCommit<'_> {
    fn drop(&mut self) {
        let mut states = self.columns.write().unwrap_or_else(PoisonError::into_inner);
        for (&position, update) in &self.updates {
            let state = states.get_mut(&position).expect("Published column commit");
            if self.committed {
                state.terrain |= update.terrain;
                if let Some(present) = update.containers {
                    state.containers = present;
                }
            }
            state.revision += 1;
        }
    }
}

pub(super) struct PreparedCheckpoint {
    metadata: Vec<u8>,
    checkpoint: Vec<u8>,
    chunks: Vec<((i32, i32), Vec<u8>)>,
    containers: Vec<((i32, i32), Option<Vec<u8>>)>,
    updates: HashMap<(i32, i32), ColumnUpdate>,
}
impl PreparedCheckpoint {
    pub(super) fn encode(
        metadata: &WorldMetadata,
        checkpoint: &Checkpoint,
        chunks: Vec<((i32, i32), Vec<u8>)>,
        containers: Vec<((i32, i32), Option<Vec<u8>>)>,
    ) -> io::Result<Self> {
        let chunks: Vec<_> = chunks
            .into_iter()
            .map(|(p, bytes)| (p, compress(&bytes)))
            .collect();
        let mut updates = HashMap::<_, ColumnUpdate>::new();
        for &(position, _) in &chunks {
            updates.entry(position).or_default().terrain = true;
        }
        let containers: Vec<_> = containers
            .into_iter()
            .map(|(position, bytes)| {
                let bytes = bytes
                    .map(|bytes| {
                        if bytes.len() > MAX_RECORD_BYTES {
                            return Err(err("Container snapshot exceeds the record size limit."));
                        }
                        Ok(compress(&bytes))
                    })
                    .transpose()?;
                updates.entry(position).or_default().containers = Some(bytes.is_some());
                Ok((position, bytes))
            })
            .collect::<io::Result<_>>()?;
        let metadata = serde_json::to_vec(metadata).map_err(err)?;
        let checkpoint = serde_json::to_vec(checkpoint).map_err(err)?;
        Ok(Self {
            metadata,
            checkpoint,
            chunks,
            containers,
            updates,
        })
    }
}

pub(crate) struct Database {
    /// Writer for gameplay checkpoints and generated section records.
    connection: Mutex<Connection>,
    /// WAL reader: chunk streaming never waits on a checkpoint transaction or its fsync.
    reader: Mutex<Connection>,
    /// Columns with a saved snapshot. Streaming skips SQLite for every other column.
    columns: RwLock<HashMap<(i32, i32), ColumnState>>,
    _lock: std::fs::File,
}
impl Database {
    fn from_connection(conn: Connection, path: &Path, lock: std::fs::File) -> io::Result<Self> {
        let reader =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        reader.busy_timeout(Duration::from_secs(5)).map_err(err)?;
        let mut columns = HashMap::<_, ColumnState>::new();
        for (table, terrain) in [("edited", true), ("containers", false)] {
            let mut query = conn
                .prepare(&format!("SELECT x,z FROM {table}"))
                .map_err(err)?;
            let positions = query
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(err)?;
            for position in positions {
                let state = columns.entry(position.map_err(err)?).or_default();
                if terrain {
                    state.terrain = true;
                } else {
                    state.containers = true;
                }
            }
        }
        Ok(Self {
            columns: RwLock::new(columns),
            connection: Mutex::new(conn),
            reader: Mutex::new(reader),
            _lock: lock,
        })
    }

    pub(super) fn create(metadata: &WorldMetadata) -> io::Result<Arc<Self>> {
        let root = root()?;
        std::fs::create_dir_all(&root)?;
        let folder = root.join(&metadata.id);
        std::fs::create_dir(&folder)?;
        let result = (|| {
            let lock = Self::lock(&folder)?;
            let path = folder.join("world.sqlite3");
            let conn = Connection::open(&path).map_err(err)?;
            Self::configure(&conn)?;
            conn.execute_batch(SCHEMA).map_err(err)?;
            conn.execute(
                "INSERT INTO records VALUES('metadata',?1)",
                [serde_json::to_vec(metadata).map_err(err)?],
            )
            .map_err(err)?;
            Self::from_connection(conn, &path, lock).map(Arc::new)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&folder);
        }
        result
    }

    pub(super) fn open(path: &Path) -> io::Result<(Arc<Self>, WorldMetadata, Checkpoint)> {
        let lock = Self::lock(path.parent().ok_or_else(|| err("Invalid world path."))?)?;
        let conn =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(err)?;
        let metadata: WorldMetadata =
            read_json(&conn, "metadata")?.ok_or_else(|| err("Missing world metadata."))?;
        metadata.validate()?;
        Self::configure(&conn)?;
        let checkpoint: Checkpoint = read_json(&conn, "checkpoint")?.unwrap_or_default();
        if let Some(player) = &checkpoint.player {
            player.inventory()?;
        }
        for drop in &checkpoint.drops {
            drop.validate()?;
        }
        if checkpoint
            .support_pending
            .iter()
            .any(|p| !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&p[1]))
        {
            return Err(err("Invalid pending support update."));
        }
        if checkpoint.pending.iter().any(|(p, s)| {
            !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&p[1])
                || crate::voxel::VoxelState::from_id(*s).is_none()
        }) {
            return Err(err("Invalid pending block edits."));
        }
        Ok((
            Arc::new(Self::from_connection(conn, path, lock)?),
            metadata,
            checkpoint,
        ))
    }

    fn lock(folder: &Path) -> io::Result<std::fs::File> {
        let path = folder.join("world.lock");
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let started = std::time::Instant::now();
        loop {
            match file.try_lock() {
                // The world may have been deleted while this waited for its lock.
                Ok(()) if !path.exists() => {
                    return Err(err("This world no longer exists."));
                }
                Ok(()) => return Ok(file),
                Err(error) => {
                    let error: io::Error = error.into();
                    if error.kind() != io::ErrorKind::WouldBlock
                        || started.elapsed() >= Duration::from_secs(5)
                    {
                        return Err(err(format!(
                            "This world is in use by another session (or locking is unavailable): {error}"
                        )));
                    }
                    // The preceding session may still be finishing background storage work.
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    fn configure(conn: &Connection) -> io::Result<()> {
        conn.busy_timeout(Duration::from_secs(5)).map_err(err)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA cache_size=-16384; PRAGMA wal_autocheckpoint=2048;").map_err(err)
    }

    /// Whether current gameplay terrain has a saved snapshot. Never touches SQLite.
    fn column(&self, x: i32, z: i32) -> ColumnState {
        self.columns
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(x, z))
            .copied()
            .unwrap_or_default()
    }
    pub(crate) fn is_edited(&self, x: i32, z: i32) -> bool {
        self.column(x, z).terrain
    }
    pub(crate) fn revision(&self, x: i32, z: i32) -> Option<u64> {
        let revision = self.column(x, z).revision;
        (revision % 2 == 0).then_some(revision)
    }

    pub(crate) fn gameplay_column(&self, x: i32, z: i32) -> io::Result<Option<GameplayColumn>> {
        let state = self.column(x, z);
        if state.revision % 2 != 0 {
            return Ok(None);
        }
        if !state.terrain && !state.containers {
            return Ok(Some(GameplayColumn {
                revision: state.revision,
                ..Default::default()
            }));
        }
        let mut reader = self.reader.lock().map_err(err)?;
        let tx = reader.transaction().map_err(err)?;
        let read = |sql: &str| -> io::Result<Option<Vec<u8>>> {
            tx.query_row(sql, params![x, z], |row| row.get(0))
                .optional()
                .map_err(err)
        };
        let terrain = read("SELECT value FROM edited WHERE x=?1 AND z=?2")?;
        let containers = read("SELECT value FROM containers WHERE x=?1 AND z=?2")?;
        tx.commit().map_err(err)?;
        Ok(Some(GameplayColumn {
            revision: state.revision,
            terrain,
            containers,
        }))
    }

    pub(super) fn checkpoint(&self, prepared: PreparedCheckpoint) -> io::Result<()> {
        let PreparedCheckpoint {
            metadata,
            checkpoint,
            chunks,
            containers,
            updates,
        } = prepared;
        let mut conn = self.connection.lock().map_err(err)?;
        let mut publication = ColumnCommit::begin(&self.columns, updates);
        let tx = conn.transaction().map_err(err)?;
        {
            let mut insert = tx.prepare_cached("INSERT INTO edited VALUES(?1,?2,?3) ON CONFLICT(x,z) DO UPDATE SET value=excluded.value").map_err(err)?;
            for ((x, z), bytes) in chunks {
                insert.execute(params![x, z, bytes]).map_err(err)?;
            }
        }
        {
            let mut insert = tx.prepare_cached("INSERT INTO containers VALUES(?1,?2,?3) ON CONFLICT(x,z) DO UPDATE SET value=excluded.value").map_err(err)?;
            for ((x, z), bytes) in containers {
                if let Some(bytes) = bytes {
                    insert.execute(params![x, z, bytes]).map_err(err)?;
                } else {
                    tx.execute("DELETE FROM containers WHERE x=?1 AND z=?2", params![x, z])
                        .map_err(err)?;
                }
            }
        }
        for (key, bytes) in [("metadata", metadata), ("checkpoint", checkpoint)] {
            tx.execute("INSERT INTO records VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,bytes]).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        // Drop publishes presence and stable versions before returning the acknowledgement.
        publication.committed = true;
        Ok(())
    }
}

impl ChunkStorage for Database {
    fn read_chunks(
        &self,
        positions: &[(i32, i32)],
    ) -> io::Result<Vec<Option<hollow_worldgen::codec::GenerationRecord>>> {
        // Release SQLite's connection before decompression.
        let records = {
            let conn = self.reader.lock().map_err(err)?;
            let mut query = conn
                .prepare_cached("SELECT stage,value FROM generated WHERE x=?1 AND z=?2")
                .map_err(err)?;
            positions
                .iter()
                .map(|&(x, z)| {
                    query
                        .query_row(params![x, z], |r| {
                            Ok((r.get::<_, u8>(0)?, r.get::<_, Vec<u8>>(1)?))
                        })
                        .optional()
                        .map_err(err)
                })
                .collect::<io::Result<Vec<_>>>()?
        };
        records
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
        let mut conn = self.connection.lock().map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        {
            let mut insert = tx.prepare_cached("INSERT INTO generated VALUES(?1,?2,?3,?4) ON CONFLICT(x,z) DO UPDATE SET stage=excluded.stage,value=excluded.value WHERE excluded.stage>=generated.stage").map_err(err)?;
            for ((x, z), stage, bytes) in records {
                insert.execute(params![x, z, stage, bytes]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }
    fn flush(&self) -> io::Result<()> {
        self.connection
            .lock()
            .map_err(err)?
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .map_err(err)
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
                .map_err(|error| err(format!("Incompatible or corrupt {key} record: {error}")))
        })
        .transpose()
}
