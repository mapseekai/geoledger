//! Local, crash-consistent object storage. Immutable content and mutable refs use
//! SQLite transactions; the application journals the separate PostGIS commit.
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use spatial_version_core::{
    Error, ObjectId, ObjectStore, PendingOperation, RepositoryState, Result, object::digest,
};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_OBJECT_BYTES: usize = 64 * 1024 * 1024;
const APPLICATION_ID: i64 = 0x53565031;
fn db_error(e: rusqlite::Error) -> Error {
    Error::Storage(e.to_string())
}

pub struct Repository {
    connection: Connection,
    _lock: File,
    pub directory: PathBuf,
}
impl Repository {
    pub fn init(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        let directory = root.join(".spatial-version");
        fs::create_dir(&directory).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Error::Conflict("repository already exists".into())
            } else {
                Error::Io(e)
            }
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
        let repo = Self::connect(directory, true)?;
        repo.connection.execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version=1;
             CREATE TABLE objects(id TEXT PRIMARY KEY, kind TEXT NOT NULL, payload BLOB NOT NULL) WITHOUT ROWID;
             CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
             CREATE TABLE reflog(sequence INTEGER PRIMARY KEY AUTOINCREMENT, branch TEXT NOT NULL,
                 old_head TEXT, new_head TEXT NOT NULL, recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);"
        )).map_err(db_error)?;
        Ok(repo)
    }
    pub fn open(root: &Path) -> Result<Self> {
        let directory = root.join(".spatial-version");
        if !directory.join("repository.sqlite").is_file() {
            return Err(Error::NotFound(format!("repository at {}", root.display())));
        }
        let repo = Self::connect(directory, false)?;
        let app: i64 = repo
            .connection
            .query_row("PRAGMA application_id", [], |r| r.get(0))
            .map_err(db_error)?;
        let version: i64 = repo
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if app != APPLICATION_ID || version != 1 {
            return Err(Error::Unsupported("repository storage format".into()));
        }
        Ok(repo)
    }
    fn connect(directory: PathBuf, create: bool) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(directory.join("lock"))?;
        lock.try_lock_exclusive().map_err(|e| {
            if e.kind() == std::io::ErrorKind::WouldBlock {
                Error::Busy
            } else {
                Error::Io(e)
            }
        })?;
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | if create {
                rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            } else {
                rusqlite::OpenFlags::empty()
            };
        let connection = Connection::open_with_flags(directory.join("repository.sqlite"), flags)
            .map_err(db_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(db_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
            )
            .map_err(db_error)?;
        Ok(Self {
            connection,
            _lock: lock,
            directory,
        })
    }
    pub fn begin(&self) -> Result<()> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(db_error)
    }
    pub fn commit(&self) -> Result<()> {
        self.connection.execute_batch("COMMIT").map_err(db_error)
    }
    pub fn rollback(&self) -> Result<()> {
        if !self.connection.is_autocommit() {
            self.connection
                .execute_batch("ROLLBACK")
                .map_err(db_error)?;
        }
        Ok(())
    }
    fn read_meta<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value: Option<String> = self
            .connection
            .query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_error)?;
        value.map(|v| Ok(serde_json::from_str(&v)?)).transpose()
    }
    fn write_meta<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.connection.execute("INSERT INTO metadata(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, serde_json::to_string(value)?]).map_err(db_error)?;
        Ok(())
    }
    pub fn state(&self) -> Result<RepositoryState> {
        let state: RepositoryState = self
            .read_meta("state")?
            .ok_or_else(|| Error::Storage("missing repository state".into()))?;
        if state.version != 1 {
            return Err(Error::Unsupported("repository state format".into()));
        }
        state.head()?;
        Ok(state)
    }
    pub fn save_state(&self, state: &RepositoryState) -> Result<()> {
        let previous: Option<RepositoryState> = self.read_meta("state")?;
        let old_head = previous.as_ref().map(|s| s.head().cloned()).transpose()?;
        let new_head = state.head()?;
        if old_head.as_ref() != Some(new_head)
            || previous.as_ref().is_none_or(|s| s.branch != state.branch)
        {
            self.connection
                .execute(
                    "INSERT INTO reflog(branch,old_head,new_head) VALUES(?1,?2,?3)",
                    params![
                        state.branch,
                        old_head.as_ref().map(ObjectId::as_str),
                        new_head.as_str()
                    ],
                )
                .map_err(db_error)?;
        }
        self.write_meta("state", state)
    }
    pub fn pending(&self) -> Result<Option<PendingOperation>> {
        self.read_meta("pending")
    }
    pub fn prepare(&self, operation: &PendingOperation) -> Result<()> {
        if self.pending()?.is_some() {
            return Err(Error::Recovery("an operation is already pending".into()));
        }
        self.write_meta("pending", operation)
    }
    pub fn clear_pending(&self) -> Result<()> {
        self.connection
            .execute("DELETE FROM metadata WHERE key='pending'", [])
            .map_err(db_error)?;
        Ok(())
    }
    pub fn reflog(&self, limit: usize) -> Result<Vec<serde_json::Value>> {
        let mut query = self.connection.prepare("SELECT sequence,branch,old_head,new_head,recorded_at FROM reflog ORDER BY sequence DESC LIMIT ?1").map_err(db_error)?;
        let rows = query.query_map([limit.min(1000) as i64], |r| Ok(serde_json::json!({
            "sequence": r.get::<_,i64>(0)?, "branch": r.get::<_,String>(1)?,
            "old_head": r.get::<_,Option<String>>(2)?, "new_head": r.get::<_,String>(3)?, "recorded_at": r.get::<_,String>(4)?
        }))).map_err(db_error)?;
        rows.map(|r| r.map_err(db_error)).collect()
    }
    pub fn verify_objects(&self) -> Result<u64> {
        let check: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(db_error)?;
        if check != "ok" {
            return Err(Error::Storage(format!("SQLite integrity check: {check}")));
        }
        let mut query = self
            .connection
            .prepare("SELECT id,kind,payload FROM objects")
            .map_err(db_error)?;
        let mut rows = query.query([]).map_err(db_error)?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(db_error)? {
            let id: String = row.get(0).map_err(db_error)?;
            let kind: String = row.get(1).map_err(db_error)?;
            let data: Vec<u8> = row.get(2).map_err(db_error)?;
            let bytes = zstd::bulk::decompress(&data, MAX_OBJECT_BYTES)
                .map_err(|e| Error::Storage(e.to_string()))?;
            if digest(&kind, &bytes).as_str() != id {
                return Err(Error::Storage(format!("corrupt object {id}")));
            }
            count += 1;
        }
        Ok(count)
    }
}
impl ObjectStore for Repository {
    fn put(&self, kind: &str, bytes: &[u8]) -> Result<ObjectId> {
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(Error::Invalid("object exceeds 64 MiB limit".into()));
        }
        let id = digest(kind, bytes);
        let payload = zstd::bulk::compress(bytes, 3).map_err(|e| Error::Storage(e.to_string()))?;
        self.connection
            .execute(
                "INSERT INTO objects(id,kind,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING",
                params![id.as_str(), kind, payload],
            )
            .map_err(db_error)?;
        Ok(id)
    }
    fn get(&self, id: &ObjectId, expected_kind: &str) -> Result<Vec<u8>> {
        let row: Option<(String, Vec<u8>)> = self
            .connection
            .query_row(
                "SELECT kind,payload FROM objects WHERE id=?1",
                [id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        let (kind, data) = row.ok_or_else(|| Error::NotFound(format!("object {id}")))?;
        let bytes = zstd::bulk::decompress(&data, MAX_OBJECT_BYTES)
            .map_err(|e| Error::Storage(e.to_string()))?;
        if kind != expected_kind || digest(&kind, &bytes) != *id {
            return Err(Error::Storage(format!(
                "type or checksum mismatch for {id}"
            )));
        }
        Ok(bytes)
    }
}
impl Drop for Repository {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}
