//! SQLite WAL storage. Blocking methods belong on I/O workers, never in ECS systems.
use super::decompress_record as decompress;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Mutex, PoisonError, RwLock},
    time::Duration,
};
fn err(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
const SCHEMA: &str = "
    CREATE TABLE records(key TEXT PRIMARY KEY, value BLOB NOT NULL);
    CREATE TABLE generated(x INTEGER NOT NULL,z INTEGER NOT NULL,stage INTEGER NOT NULL,value BLOB NOT NULL,PRIMARY KEY(x,z)) WITHOUT ROWID;
    CREATE TABLE application(x INTEGER NOT NULL,z INTEGER NOT NULL,value BLOB NOT NULL,PRIMARY KEY(x,z)) WITHOUT ROWID;
    CREATE TABLE edited(x INTEGER NOT NULL,z INTEGER NOT NULL,value BLOB NOT NULL,PRIMARY KEY(x,z)) WITHOUT ROWID;
";

/// Presence and a process-local revision; odd revisions denote an in-flight commit.
#[derive(Clone, Copy, Default)]
struct ColumnState {
    terrain: bool,
    application: bool,
    revision: u64,
}
#[derive(Default)]
struct ColumnUpdate {
    terrain: bool,
    application: bool,
}

/// Compressed WAL data travels to a compute worker before it is decoded.
#[derive(Default)]
pub struct StoredColumn {
    pub revision: u64,
    terrain: Option<Vec<u8>>,
    application: Option<Vec<u8>>,
}
impl StoredColumn {
    pub fn decode(self) -> io::Result<(Option<Vec<u8>>, Option<Vec<u8>>)> {
        Ok((
            self.terrain.map(|bytes| decompress(&bytes)).transpose()?,
            self.application
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
                state.application |= update.application;
            }
            state.revision += 1;
        }
    }
}

pub struct PreparedCheckpoint {
    metadata: Vec<u8>,
    checkpoint: Vec<u8>,
    updates: HashMap<(i32, i32), ColumnUpdate>,
}
pub struct EncodedColumn {
    pub position: (i32, i32),
    pub terrain: Option<Vec<u8>>,
    pub application: Option<Vec<u8>>,
}
impl PreparedCheckpoint {
    pub fn new(
        metadata: Vec<u8>,
        checkpoint: Vec<u8>,
        columns: Vec<((i32, i32), bool, bool)>,
    ) -> Self {
        Self {
            metadata,
            checkpoint,
            updates: columns
                .into_iter()
                .map(|(position, terrain, application)| {
                    (
                        position,
                        ColumnUpdate {
                            terrain,
                            application,
                        },
                    )
                })
                .collect(),
        }
    }
}

pub struct SqliteStore {
    /// Writer for application checkpoints and generated section records.
    connection: Mutex<Connection>,
    /// WAL reader: chunk streaming never waits on a checkpoint transaction or its fsync.
    reader: Mutex<Connection>,
    /// Columns with a saved snapshot. Streaming skips SQLite for every other column.
    columns: RwLock<HashMap<(i32, i32), ColumnState>>,
    _lock: std::fs::File,
}
impl SqliteStore {
    fn from_connection(conn: Connection, path: &Path, lock: std::fs::File) -> io::Result<Self> {
        let reader =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
        reader.busy_timeout(Duration::from_secs(5)).map_err(err)?;
        let mut columns = HashMap::<_, ColumnState>::new();
        for (table, terrain) in [("edited", true), ("application", false)] {
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
                    state.application = true;
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

    pub fn create(path: &Path, metadata: &[u8]) -> io::Result<Self> {
        let lock = Self::lock(path.parent().ok_or_else(|| err("Invalid world path"))?)?;
        let conn = Connection::open(path).map_err(err)?;
        Self::configure(&conn)?;
        conn.execute_batch(SCHEMA).map_err(err)?;
        conn.execute("INSERT INTO records VALUES('metadata',?1)", [metadata])
            .map_err(err)?;
        Self::from_connection(conn, path, lock)
    }
    pub fn open(path: &Path) -> io::Result<Self> {
        let lock = Self::lock(path.parent().ok_or_else(|| err("Invalid world path"))?)?;
        let conn =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(err)?;
        Self::configure(&conn)?;
        Self::from_connection(conn, path, lock)
    }
    pub fn record(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.reader
            .lock()
            .map_err(err)?
            .query_row("SELECT value FROM records WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(err)
    }
    pub fn lock(folder: &Path) -> io::Result<std::fs::File> {
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

    /// Whether current application terrain has a saved snapshot. Never touches SQLite.
    fn column(&self, x: i32, z: i32) -> ColumnState {
        self.columns
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(x, z))
            .copied()
            .unwrap_or_default()
    }
    pub fn is_edited(&self, x: i32, z: i32) -> bool {
        self.column(x, z).terrain
    }
    pub fn revision(&self, x: i32, z: i32) -> Option<u64> {
        let revision = self.column(x, z).revision;
        (revision % 2 == 0).then_some(revision)
    }

    pub fn load_column(&self, x: i32, z: i32) -> io::Result<Option<StoredColumn>> {
        let state = self.column(x, z);
        if state.revision % 2 != 0 {
            return Ok(None);
        }
        if !state.terrain && !state.application {
            return Ok(Some(StoredColumn {
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
        let application = read("SELECT value FROM application WHERE x=?1 AND z=?2")?;
        tx.commit().map_err(err)?;
        Ok(Some(StoredColumn {
            revision: state.revision,
            terrain,
            application,
        }))
    }

    pub fn checkpoint(
        &self,
        prepared: PreparedCheckpoint,
        batches: impl Iterator<Item = io::Result<Vec<EncodedColumn>>>,
    ) -> io::Result<()> {
        let PreparedCheckpoint {
            metadata,
            checkpoint,
            updates,
        } = prepared;
        let mut conn = self.connection.lock().map_err(err)?;
        let mut publication = ColumnCommit::begin(&self.columns, updates);
        let tx = conn.transaction().map_err(err)?;
        {
            let mut terrain = tx.prepare_cached("INSERT INTO edited VALUES(?1,?2,?3) ON CONFLICT(x,z) DO UPDATE SET value=excluded.value").map_err(err)?;
            let mut application = tx.prepare_cached("INSERT INTO application VALUES(?1,?2,?3) ON CONFLICT(x,z) DO UPDATE SET value=excluded.value").map_err(err)?;
            for batch in batches {
                for column in batch? {
                    let (x, z) = column.position;
                    if let Some(bytes) = column.terrain {
                        terrain.execute(params![x, z, bytes]).map_err(err)?;
                    }
                    if let Some(bytes) = column.application {
                        application.execute(params![x, z, bytes]).map_err(err)?;
                    }
                }
            }
        }
        for (key, bytes) in [("metadata", metadata), ("checkpoint", checkpoint)] {
            tx.execute("INSERT INTO records VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,bytes]).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        publication.committed = true;
        Ok(())
    }
}

impl SqliteStore {
    pub fn read_provider(
        &self,
        positions: &[(i32, i32)],
    ) -> io::Result<Vec<Option<(u8, Vec<u8>)>>> {
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
        Ok(records)
    }
    pub fn write_provider(&self, chunks: Vec<((i32, i32), u8, Vec<u8>)>) -> io::Result<()> {
        let mut conn = self.connection.lock().map_err(err)?;
        let tx = conn.transaction().map_err(err)?;
        {
            let mut insert = tx.prepare_cached("INSERT INTO generated VALUES(?1,?2,?3,?4) ON CONFLICT(x,z) DO UPDATE SET stage=excluded.stage,value=excluded.value WHERE excluded.stage>=generated.stage").map_err(err)?;
            for ((x, z), stage, bytes) in chunks {
                insert.execute(params![x, z, stage, bytes]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }
    pub fn flush(&self) -> io::Result<()> {
        self.connection
            .lock()
            .map_err(err)?
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
            .map_err(err)
    }
}
