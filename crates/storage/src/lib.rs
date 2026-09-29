//! Local, crash-consistent object storage. Immutable content and mutable refs use
//! SQLite transactions; the application journals the separate PostGIS commit.
use fs2::FileExt;
use geoledger_core::{
    Error, ObjectId, ObjectStore, PendingOperation, RepositoryState, Result, STATE_VERSION,
    object::digest, schema,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};

const STORAGE_VERSION: u32 = 5;
const CONFLICT_TABLE: &str = "CREATE TABLE conflicts(dataset TEXT NOT NULL, key TEXT NOT NULL, refs TEXT NOT NULL, PRIMARY KEY(dataset,key)) WITHOUT ROWID;
CREATE TABLE conflict_stats(singleton INTEGER PRIMARY KEY CHECK(singleton=1), count INTEGER NOT NULL CHECK(count>=0));
INSERT INTO conflict_stats VALUES(1,0);
CREATE TRIGGER conflict_insert AFTER INSERT ON conflicts BEGIN UPDATE conflict_stats SET count=count+1 WHERE singleton=1; END;
CREATE TRIGGER conflict_delete AFTER DELETE ON conflicts BEGIN UPDATE conflict_stats SET count=count-1 WHERE singleton=1; END;";

const MAX_OBJECT_BYTES: usize = 64 * 1024 * 1024;
const APPLICATION_ID: i64 = 0x474c4433; // ASCII GLD3.
const REPOSITORY_DIRECTORY: &str = ".geoledger";

fn validate_state(state: &RepositoryState) -> Result<()> {
    if state.version != STATE_VERSION {
        return Err(Error::Unsupported(
            "GeoLedger repository state format".into(),
        ));
    }
    for binding in state.bindings.values() {
        schema::validate_format(&binding.schema)?;
    }
    state.head()?;
    Ok(())
}

fn db_error(e: rusqlite::Error) -> Error {
    Error::storage_source(e.to_string(), e)
}

pub struct Repository {
    connection: Connection,
    compressor: RefCell<zstd::bulk::Compressor<'static>>,
    decompressor: RefCell<zstd::bulk::Decompressor<'static>>,
    _lock: File,
    pub directory: PathBuf,
}
impl Repository {
    pub fn init(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        let directory = root.join(REPOSITORY_DIRECTORY);
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
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={STORAGE_VERSION};
             CREATE TABLE objects(id TEXT PRIMARY KEY, kind TEXT NOT NULL, payload BLOB NOT NULL) WITHOUT ROWID;
             CREATE TABLE metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
             CREATE TABLE reflog(sequence INTEGER PRIMARY KEY AUTOINCREMENT, branch TEXT NOT NULL,
                 old_head TEXT, new_head TEXT NOT NULL, recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);"
        )).map_err(db_error)?;
        repo.connection
            .execute_batch(CONFLICT_TABLE)
            .map_err(db_error)?;
        Ok(repo)
    }
    pub fn open(root: &Path) -> Result<Self> {
        let directory = root.join(REPOSITORY_DIRECTORY);
        if !directory.join("repository.sqlite").is_file() {
            return Err(Error::NotFound(format!("repository at {}", root.display())));
        }
        Self::connect(directory, false)
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
            if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
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
        if !create {
            // Validate the current layout before applying any database settings.
            let application: i64 = connection
                .query_row("PRAGMA application_id", [], |r| r.get(0))
                .map_err(db_error)?;
            let version: i64 = connection
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .map_err(db_error)?;
            if application != APPLICATION_ID || version != i64::from(STORAGE_VERSION) {
                return Err(Error::Unsupported(
                    "repository storage format; initialize a new development repository".into(),
                ));
            }
        }
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(db_error)?;
        connection.set_prepared_statement_cache_capacity(32);
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
                 PRAGMA cache_size=-65536; PRAGMA mmap_size=268435456;
                 PRAGMA temp_store=FILE; PRAGMA temp.cache_size=-16384;",
            )
            .map_err(db_error)?;
        Ok(Self {
            connection,
            compressor: RefCell::new(zstd::bulk::Compressor::new(3)?),
            decompressor: RefCell::new(zstd::bulk::Decompressor::new()?),
            _lock: lock,
            directory,
        })
    }
    pub fn begin(&self) -> Result<()> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(db_error)
    }
    /// Stage a full scan in an append-oriented temporary table, then insert in
    /// hash order. This avoids repeatedly spilling randomly modified main pages.
    /// The caller's transaction still owns durability and the journal boundary.
    pub fn bulk_write<T>(&self, write: impl FnOnce(&dyn ObjectStore) -> Result<T>) -> Result<T> {
        if self.connection.is_autocommit() {
            return Err(Error::Storage("bulk write requires a transaction".into()));
        }
        self.connection
            .execute_batch("SAVEPOINT bulk_objects")
            .map_err(db_error)?;
        let result = (|| {
            self.connection.execute_batch(
                "CREATE TEMP TABLE bulk_objects(id TEXT PRIMARY KEY, kind TEXT NOT NULL, payload BLOB NOT NULL);"
            ).map_err(db_error)?;
            let value = write(&BulkStore(self))?;
            self.connection
                .execute_batch(
                    "INSERT OR IGNORE INTO objects(id,kind,payload)
                 SELECT id,kind,payload FROM temp.bulk_objects ORDER BY id;
                 DROP TABLE temp.bulk_objects;",
                )
                .map_err(db_error)?;
            Ok(value)
        })();
        if result.is_err() {
            self.connection
                .execute_batch("ROLLBACK TO bulk_objects")
                .map_err(db_error)?;
        }
        self.connection
            .execute_batch("RELEASE bulk_objects")
            .map_err(db_error)?;
        result
    }
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        let size = zstd::zstd_safe::get_frame_content_size(data)
            .map_err(|_| Error::Storage("invalid compressed object header".into()))?
            .unwrap_or(MAX_OBJECT_BYTES as u64);
        if size > MAX_OBJECT_BYTES as u64 {
            return Err(Error::Storage("object exceeds decompression limit".into()));
        }
        self.decompressor
            .borrow_mut()
            .decompress(data, size as usize)
            .map_err(|e| Error::storage_source(e.to_string(), e))
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
        value
            .map(|v| {
                serde_json::from_str(&v)
                    .map_err(|e| Error::storage_source(format!("decode metadata {key}"), e))
            })
            .transpose()
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
        validate_state(&state)?;
        let owner: Option<Option<(ObjectId, ObjectId, ObjectId)>> =
            self.read_meta("conflict_owner")?;
        if owner.flatten() != conflict_owner(&state)
            || (self.conflict_count()? != 0 && state.merging.is_none())
        {
            return Err(Error::Storage(
                "conflict index/merge identity mismatch".into(),
            ));
        }
        Ok(state)
    }
    pub fn save_state(&self, state: &RepositoryState) -> Result<()> {
        validate_state(state)?;
        if state.merging.is_none() {
            self.connection
                .execute("DELETE FROM conflicts", [])
                .map_err(db_error)?;
        }
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
        self.write_meta("conflict_owner", &conflict_owner(state))?;
        self.write_meta("state", state)
    }
    pub fn pending(&self) -> Result<Option<PendingOperation>> {
        let pending: Option<PendingOperation> = self.read_meta("pending")?;
        if let Some(operation) = &pending {
            validate_state(&operation.after)?;
        }
        Ok(pending)
    }
    pub fn prepare(&self, operation: &PendingOperation) -> Result<()> {
        validate_state(&operation.after)?;
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
    pub fn insert_conflict(&self, conflict: &geoledger_core::Conflict) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(Error::Storage("conflicts require a transaction".into()));
        }
        if [&conflict.base, &conflict.ours, &conflict.theirs]
            .into_iter()
            .flatten()
            .any(|r| r.key != conflict.key)
        {
            return Err(Error::Storage("conflict record key mismatch".into()));
        }
        let refs = ConflictRefs {
            base: conflict
                .base
                .as_ref()
                .map(|r| geoledger_core::save(self, "record/v3", r))
                .transpose()?,
            ours: conflict
                .ours
                .as_ref()
                .map(|r| geoledger_core::save(self, "record/v3", r))
                .transpose()?,
            theirs: conflict
                .theirs
                .as_ref()
                .map(|r| geoledger_core::save(self, "record/v3", r))
                .transpose()?,
            fields: conflict.fields.clone(),
        };
        self.connection
            .execute(
                "INSERT INTO conflicts(dataset,key,refs) VALUES(?1,?2,?3)",
                params![
                    conflict.dataset,
                    conflict.key,
                    serde_json::to_string(&refs)?
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn conflict_count(&self) -> Result<usize> {
        self.connection
            .query_row(
                "SELECT count FROM conflict_stats WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(db_error)
    }
    fn decode_conflict(
        &self,
        dataset: String,
        key: String,
        refs: String,
    ) -> Result<geoledger_core::Conflict> {
        let refs: ConflictRefs = serde_json::from_str(&refs)
            .map_err(|e| Error::storage_source("decode conflict references", e))?;
        let read = |id: Option<ObjectId>| -> Result<Option<geoledger_core::Record>> {
            let record: Option<geoledger_core::Record> = id
                .as_ref()
                .map(|id| geoledger_core::load(self, "record/v3", id))
                .transpose()?;
            if record.as_ref().is_some_and(|r| r.key != key) {
                return Err(Error::Storage("conflict record key mismatch".into()));
            }
            Ok(record)
        };
        let (base, ours, theirs) = (read(refs.base)?, read(refs.ours)?, read(refs.theirs)?);
        Ok(geoledger_core::Conflict {
            dataset,
            key,
            base,
            ours,
            theirs,
            fields: refs.fields,
        })
    }
    pub fn conflict(&self, dataset: &str, key: &str) -> Result<geoledger_core::Conflict> {
        let refs: String = self
            .connection
            .query_row(
                "SELECT refs FROM conflicts WHERE dataset=?1 AND key=?2",
                params![dataset, key],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| Error::NotFound("unresolved conflict".into()))?;
        self.decode_conflict(dataset.into(), key.into(), refs)
    }
    pub fn conflicts_page(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<geoledger_core::Conflict>> {
        let mut query = self
            .connection
            .prepare(
                "SELECT dataset,key,refs FROM conflicts ORDER BY dataset,key LIMIT ?1 OFFSET ?2",
            )
            .map_err(db_error)?;
        let mut rows = query
            .query(params![
                limit.clamp(1, 1000) as i64,
                i64::try_from(offset).unwrap_or(i64::MAX)
            ])
            .map_err(db_error)?;
        let mut result = Vec::new();
        let mut bytes = 0usize;
        while let Some(row) = rows.next().map_err(db_error)? {
            let conflict = self.decode_conflict(
                row.get(0).map_err(db_error)?,
                row.get(1).map_err(db_error)?,
                row.get(2).map_err(db_error)?,
            )?;
            for record in [&conflict.base, &conflict.ours, &conflict.theirs]
                .into_iter()
                .flatten()
            {
                bytes = bytes.saturating_add(record.payload_bytes());
            }
            result.push(conflict);
            if bytes >= 8 * 1024 * 1024 {
                break;
            }
        }
        Ok(result)
    }
    pub fn remove_conflict(&self, dataset: &str, key: &str) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(Error::Storage("resolution requires a transaction".into()));
        }
        if self
            .connection
            .execute(
                "DELETE FROM conflicts WHERE dataset=?1 AND key=?2",
                params![dataset, key],
            )
            .map_err(db_error)?
            != 1
        {
            return Err(Error::NotFound("unresolved conflict".into()));
        }
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
        let actual_conflicts: usize = self
            .connection
            .query_row("SELECT count(*) FROM conflicts", [], |r| r.get(0))
            .map_err(db_error)?;
        if actual_conflicts != self.conflict_count()? {
            return Err(Error::Storage("conflict count mismatch".into()));
        }
        let mut conflicts = self
            .connection
            .prepare("SELECT dataset,key,refs FROM conflicts ORDER BY dataset,key")
            .map_err(db_error)?;
        let mut rows = conflicts.query([]).map_err(db_error)?;
        while let Some(row) = rows.next().map_err(db_error)? {
            self.decode_conflict(
                row.get(0).map_err(db_error)?,
                row.get(1).map_err(db_error)?,
                row.get(2).map_err(db_error)?,
            )?;
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
            let bytes = self.decompress(&data)?;
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
        self.put_into(kind, bytes, false)
    }
    fn get(&self, id: &ObjectId, expected_kind: &str) -> Result<Vec<u8>> {
        self.get_from(id, expected_kind, false)
    }
}
impl Repository {
    fn put_into(&self, kind: &str, bytes: &[u8], bulk: bool) -> Result<ObjectId> {
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(Error::Invalid("object exceeds 64 MiB limit".into()));
        }
        let id = digest(kind, bytes);
        let payload = self
            .compressor
            .borrow_mut()
            .compress(bytes)
            .map_err(|e| Error::storage_source(e.to_string(), e))?;
        self.connection
            .prepare_cached(if bulk {
                "INSERT INTO temp.bulk_objects(id,kind,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING"
            } else {
                "INSERT INTO objects(id,kind,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING"
            })
            .map_err(db_error)?
            .execute(params![id.as_str(), kind, payload])
            .map_err(db_error)?;
        Ok(id)
    }
    fn get_from(&self, id: &ObjectId, expected_kind: &str, bulk: bool) -> Result<Vec<u8>> {
        let row: Option<(String, Vec<u8>)> = self
            .connection
            .prepare_cached(if bulk {
                "SELECT kind,payload FROM temp.bulk_objects WHERE id=?1 UNION ALL SELECT kind,payload FROM objects WHERE id=?1 LIMIT 1"
            } else {
                "SELECT kind,payload FROM objects WHERE id=?1"
            })
            .map_err(db_error)?
            .query_row([id.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(db_error)?;
        let (kind, data) = row.ok_or_else(|| Error::NotFound(format!("object {id}")))?;
        let bytes = self.decompress(&data)?;
        if kind != expected_kind || digest(&kind, &bytes) != *id {
            return Err(Error::Storage(format!(
                "type or checksum mismatch for {id}"
            )));
        }
        Ok(bytes)
    }
}
struct BulkStore<'a>(&'a Repository);
impl ObjectStore for BulkStore<'_> {
    fn put(&self, kind: &str, bytes: &[u8]) -> Result<ObjectId> {
        self.0.put_into(kind, bytes, true)
    }
    fn get(&self, id: &ObjectId, expected_kind: &str) -> Result<Vec<u8>> {
        self.0.get_from(id, expected_kind, true)
    }
}
impl Drop for Repository {
    fn drop(&mut self) {
        let _ = self.rollback();
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ConflictRefs {
    base: Option<ObjectId>,
    ours: Option<ObjectId>,
    theirs: Option<ObjectId>,
    fields: Vec<String>,
}

fn conflict_owner(state: &RepositoryState) -> Option<(ObjectId, ObjectId, ObjectId)> {
    state
        .merging
        .as_ref()
        .map(|m| (m.base.clone(), m.ours.clone(), m.theirs.clone()))
}
