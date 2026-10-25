//! Portable transaction boundary; SQL is shared, locking and spatial indexes are explicit dialect choices.
mod audit;
mod backend;
mod dataset;
mod history;
pub(crate) mod pgtls;
pub(crate) mod portable;
pub(crate) mod postgres;
mod project;
mod publication;
mod query;
mod schema;
mod tables;
mod workspace;
use crate::repository::*;
use crate::{Error, FORMAT_VERSION, Result, Storage};
use rusqlite::functions::FunctionFlags;
use serde_json::value::RawValue;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) trait Parameter {
    fn value(&self) -> Cell;
}
impl Parameter for str {
    fn value(&self) -> Cell {
        Cell::Text(self.to_owned())
    }
}
impl Parameter for String {
    fn value(&self) -> Cell {
        Cell::Text(self.clone())
    }
}
impl<T: Parameter + ?Sized> Parameter for &T {
    fn value(&self) -> Cell {
        (*self).value()
    }
}
impl<T: Parameter> Parameter for Option<T> {
    fn value(&self) -> Cell {
        self.as_ref().map_or(Cell::Null, Parameter::value)
    }
}
impl Parameter for i64 {
    fn value(&self) -> Cell {
        Cell::Integer(*self)
    }
}
impl Parameter for i32 {
    fn value(&self) -> Cell {
        Cell::Integer(i64::from(*self))
    }
}
impl Parameter for bool {
    fn value(&self) -> Cell {
        Cell::Bool(*self)
    }
}
impl Parameter for f64 {
    fn value(&self) -> Cell {
        Cell::Real(*self)
    }
}
enum Backend {
    Sqlite(geoledger_spatialite::Connection),
    Postgis(postgres::Client),
}
pub(crate) struct Client {
    backend: Backend,
    deadline: Instant,
    transaction: bool,
    result_limit: usize,
    response_reservation: Option<crate::ResponseReservation>,
    dataset_write_allowed: Arc<AtomicBool>,
}
impl Client {
    pub fn open(storage: &Storage, pool: &Arc<postgres::Pool>, timeout: Duration) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Error::new(400, "invalid deadline"))?;
        let dataset_write_allowed = Arc::new(AtomicBool::new(false));
        let backend = match storage {
            Storage::Postgis(dsn) => Backend::Postgis(pool.connect(dsn, timeout)?),
            Storage::Sqlite(path) => {
                let c = rusqlite::Connection::open(path).map_err(sqlite_error)?;
                let native = geoledger_spatialite::load_configured(c).map_err(|e| {
                    Error::new(500, "cannot load SpatiaLite; install mod_spatialite or set GL_SPATIALITE_EXTENSION to its trusted library path").caused_by(e)
                })?;
                let c = native.read().map_err(sqlite_error)?;
                c.busy_timeout(timeout).map_err(sqlite_error)?;
                c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
                    .map_err(sqlite_error)?;
                c.progress_handler(1000, Some(move || Instant::now() >= deadline))
                    .map_err(sqlite_error)?;
                let flags = FunctionFlags::SQLITE_UTF8
                    | FunctionFlags::SQLITE_DETERMINISTIC
                    | FunctionFlags::SQLITE_INNOCUOUS;
                c.create_scalar_function("gl_json_field", 2, flags, |ctx| {
                    let source: Option<String> = ctx.get(0usize)?;
                    let key: String = ctx.get(1usize)?;
                    let Some(source) = source else {
                        return Ok(None::<String>);
                    };
                    // Borrow JSON fragments: large property arrays need no Value tree or reserialization.
                    let fields: Option<HashMap<String, &RawValue>> = serde_json::from_str(&source)
                        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
                    let raw = fields
                        .as_ref()
                        .and_then(|fields| fields.get(&key))
                        .map(|v| v.get());
                    match raw {
                        None | Some("null") => Ok(None),
                        Some(raw) if raw.starts_with('"') => serde_json::from_str::<String>(raw)
                            .map(Some)
                            .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e))),
                        Some(raw) => Ok(Some(raw.to_owned())),
                    }
                })
                .map_err(sqlite_error)?;
                c.create_scalar_function("gl_spatial_collection", 1, flags, |ctx| {
                    let source: Option<String> = ctx.get(0usize)?;
                    source
                        .map(|s| crate::geometry::spatial_collection(&s))
                        .transpose()
                        .map(Option::flatten)
                        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))
                })
                .map_err(sqlite_error)?;
                c.create_scalar_function("gl_check_empty_geometry", 1, flags, |ctx| {
                    let source: Option<String> = ctx.get(0usize)?;
                    let Some(source) = source else { return Ok(None::<String>); };
                    let value: serde_json::Value = serde_json::from_str(&source)
                        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
                    fn has_position(v: &serde_json::Value) -> bool {
                        match v {
                            serde_json::Value::Array(a) => a.iter().any(|v| v.is_number() || has_position(v)),
                            serde_json::Value::Object(o) => ["coordinates", "geometries"].iter().any(|k| o.get(*k).is_some_and(has_position)),
                            _ => false,
                        }
                    }
                    if has_position(&value) {
                        return Err(rusqlite::Error::UserFunctionError(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "SpatiaLite cannot represent this geometry; coordinate dimensions must be consistent within a feature",
                        ))));
                    }
                    Ok(None::<String>)
                }).map_err(sqlite_error)?;
                let allowed = dataset_write_allowed.clone();
                c.create_scalar_function(
                    "gl_dataset_write_allowed",
                    0,
                    FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS,
                    move |_| Ok(allowed.load(Ordering::Relaxed)),
                )
                .map_err(sqlite_error)?;
                drop(c);
                Backend::Sqlite(native)
            }
        };
        Ok(Self {
            backend,
            deadline,
            transaction: false,
            result_limit: usize::MAX,
            response_reservation: None,
            dataset_write_allowed,
        })
    }
    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(Error::new(
                504,
                "operation deadline exceeded; confirm publication using the original request_id",
            ))
        } else {
            Ok(())
        }
    }
    pub fn query(&mut self, sql: &str, params: &[&dyn Parameter]) -> Result<Vec<Row>> {
        self.check()?;
        let values: Vec<_> = params.iter().map(|p| p.value()).collect();
        match &mut self.backend {
            Backend::Sqlite(c) => {
                let c = c.read().map_err(sqlite_error)?;
                c.busy_timeout(self.deadline.saturating_duration_since(Instant::now()))
                    .map_err(sqlite_error)?;
                let mut stmt = c.prepare_cached(sql).map_err(sqlite_error)?;
                for (i, v) in values.iter().enumerate() {
                    if let Some(index) = stmt
                        .parameter_index(&format!("${}", i + 1))
                        .map_err(sqlite_error)?
                    {
                        use rusqlite::types::Value as V;
                        let v = match v {
                            Cell::Null => V::Null,
                            Cell::Text(s) => V::Text(s.clone()),
                            Cell::Integer(v) => V::Integer(*v),
                            Cell::Real(v) => V::Real(*v),
                            Cell::Bool(v) => V::Integer(i64::from(*v)),
                        };
                        stmt.raw_bind_parameter(index, v).map_err(sqlite_error)?;
                    }
                }
                let count = stmt.column_count();
                let mut rows = stmt.raw_query();
                let mut out = Vec::new();
                let mut result_bytes = 0usize;
                while let Some(row) = rows.next().map_err(sqlite_error)? {
                    let mut cells = Vec::with_capacity(count);
                    for i in 0..count {
                        use rusqlite::types::ValueRef as V;
                        cells.push(match row.get_ref(i).map_err(sqlite_error)? {
                            V::Null => Cell::Null,
                            V::Integer(v) => Cell::Integer(v),
                            V::Real(v) => Cell::Real(v),
                            V::Text(s) => Cell::Text(
                                std::str::from_utf8(s)
                                    .map_err(|_| Error::new(500, "invalid stored text"))?
                                    .to_owned(),
                            ),
                            _ => return Err(Error::new(500, "unexpected storage type")),
                        });
                    }
                    for cell in &cells {
                        result_bytes = result_bytes.saturating_add(match cell {
                            Cell::Text(s) => s.len(),
                            _ => 8,
                        });
                    }
                    if result_bytes > self.result_limit {
                        return Err(Error::new(413, "database result batch exceeds byte budget"));
                    }
                    if let Some(reserve) = &self.response_reservation {
                        reserve(result_bytes)?;
                    }
                    out.push(Row(cells));
                }
                Ok(out)
            }
            Backend::Postgis(c) => {
                use tokio_postgres::types::{ToSql, Type};
                let values: Vec<Box<dyn ToSql + Sync>> = values
                    .into_iter()
                    .map(|v| -> Box<dyn ToSql + Sync> {
                        match v {
                            Cell::Null => Box::new(AnyNull),
                            Cell::Text(s) => Box::new(s),
                            Cell::Integer(v) => Box::new(v),
                            Cell::Real(v) => Box::new(v),
                            Cell::Bool(v) => Box::new(v),
                        }
                    })
                    .collect();
                let refs: Vec<_> = values.iter().map(|v| v.as_ref()).collect();
                c.query(sql, &refs)?
                    .into_iter()
                    .map(|row| {
                        let mut cells = Vec::new();
                        for (i, col) in row.columns().iter().enumerate() {
                            cells.push(
                                match *col.type_() {
                                    Type::INT8 => {
                                        row.try_get::<_, Option<i64>>(i)?.map(Cell::Integer)
                                    }
                                    Type::INT4 => row
                                        .try_get::<_, Option<i32>>(i)?
                                        .map(|v| Cell::Integer(v.into())),
                                    Type::BOOL => {
                                        row.try_get::<_, Option<bool>>(i)?.map(Cell::Bool)
                                    }
                                    Type::FLOAT8 => {
                                        row.try_get::<_, Option<f64>>(i)?.map(Cell::Real)
                                    }
                                    _ => row.try_get::<_, Option<String>>(i)?.map(Cell::Text),
                                }
                                .unwrap_or(Cell::Null),
                            );
                        }
                        Ok(Row(cells))
                    })
                    .collect()
            }
        }
    }
    pub fn query_one(&mut self, sql: &str, p: &[&dyn Parameter]) -> Result<Row> {
        let mut rows = self.query(sql, p)?;
        if rows.len() != 1 {
            return Err(Error::new(500, "unexpected storage result"));
        }
        Ok(rows.remove(0))
    }
    pub fn query_opt(&mut self, sql: &str, p: &[&dyn Parameter]) -> Result<Option<Row>> {
        let mut rows = self.query(sql, p)?;
        if rows.len() > 1 {
            return Err(Error::new(500, "unexpected storage result"));
        }
        Ok(rows.pop())
    }
    pub fn execute(&mut self, sql: &str, p: &[&dyn Parameter]) -> Result<()> {
        self.query(sql, p).map(|_| ())
    }
    pub fn batch_execute(&mut self, sql: &str) -> Result<()> {
        self.check()?;
        match &mut self.backend {
            Backend::Sqlite(c) => {
                let c = c.read().map_err(sqlite_error)?;
                c.busy_timeout(self.deadline.saturating_duration_since(Instant::now()))
                    .map_err(sqlite_error)?;
                c.execute_batch(sql).map_err(sqlite_error)
            }
            Backend::Postgis(c) => c.batch_execute(sql),
        }
    }
    fn sqlite_admission(
        &self,
        connection: &geoledger_spatialite::Connection,
        sql: &str,
    ) -> Result<()> {
        self.check()?;
        // Retry only lock admission and WAL initialization, never business SQL or COMMIT.
        // SQLite's busy handler sleeps inside execute_batch. Holding the
        // process-wide native access guard while sleeping would prevent
        // unrelated databases from initializing/closing their connections.
        loop {
            let result = {
                let access = connection.read().map_err(sqlite_error)?;
                access.busy_timeout(Duration::ZERO).map_err(sqlite_error)?;
                access.execute_batch(sql)
            };
            match result {
                Ok(()) => break,
                Err(error)
                    if error.sqlite_error_code().is_some_and(|code| {
                        matches!(
                            code,
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                    }) =>
                {
                    // Preserve SQLITE_BUSY at lock expiry, while returning
                    // a deadline error if no database work was ever attempted.
                    // The native guard is gone before the bounded wait.
                    std::thread::sleep(
                        self.deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(5)),
                    );
                    if Instant::now() >= self.deadline {
                        return Err(sqlite_error(error));
                    }
                }
                Err(error) => return Err(sqlite_error(error)),
            }
        }
        Ok(())
    }
    pub fn transaction(mut self, read_only: bool) -> Result<SqlTransaction> {
        if read_only && matches!(self.backend, Backend::Sqlite(_)) {
            self.batch_execute("PRAGMA query_only=ON")?;
        }
        if let Backend::Sqlite(connection) = &self.backend
            && !read_only
        {
            self.sqlite_admission(connection, "BEGIN IMMEDIATE")?;
        } else {
            self.batch_execute(match self.backend {
                Backend::Postgis(_) if read_only => "BEGIN ISOLATION LEVEL REPEATABLE READ",
                _ => "BEGIN",
            })?;
        }
        self.transaction = true;
        Ok(SqlTransaction(self, TransactionState::default()))
    }
    /// Initialize a fresh current-format database or validate an existing one.
    pub fn migrate(self) -> Result<()> {
        let sqlite = matches!(self.backend, Backend::Sqlite(_));
        if let Backend::Sqlite(connection) = &self.backend {
            self.sqlite_admission(connection, "PRAGMA journal_mode=WAL")?;
        }
        let mut t = self.transaction(false)?;
        if !sqlite {
            t.query_one("SELECT pg_advisory_xact_lock(7881,4)::text", &[])?;
        }
        let exists = if sqlite {
            t.query_opt(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_format'",
                &[],
            )?
            .is_some()
        } else {
            t.query_one("SELECT to_regclass('public.gl_format') IS NOT NULL", &[])?
                .get::<_, bool>(0usize)?
        };
        if !exists {
            if sqlite {
                let initialized = t
                    .query_one("SELECT InitSpatialMetaData(0, 'WGS84')", &[])?
                    .get::<_, i32>(0usize)?;
                if initialized != 1 {
                    return Err(Error::new(500, "cannot initialize SpatiaLite metadata"));
                }
            }
            t.batch_execute(if sqlite {
                include_str!("sqlite.sql")
            } else {
                include_str!("postgis.sql")
            })?;
            if sqlite {
                for table in ["gl_history", "gl_workspace_changes"] {
                    for (column, dimension) in [("geom", "XY"), ("geom_z", "XYZ")] {
                        for sql in [
                            format!(
                                "SELECT RecoverGeometryColumn('{table}','{column}',0,'GEOMETRY','{dimension}')"
                            ),
                            format!("SELECT CreateSpatialIndex('{table}','{column}')"),
                        ] {
                            if t.query_one(&sql, &[])?.get::<_, i32>(0usize)? != 1 {
                                return Err(Error::new(
                                    500,
                                    "cannot register SpatiaLite geometry/index",
                                ));
                            }
                        }
                    }
                }
            }
            t.commit()?;
            return Ok(());
        }
        let version = t
            .query_one("SELECT version FROM gl_format WHERE singleton=true", &[])?
            .get::<_, i32>(0usize)?;
        if version != FORMAT_VERSION {
            return Err(Error::new(
                409,
                "storage format must match this server; initialize a fresh database",
            ));
        }
        if !sqlite {
            let stored = t
                .query_one(
                    "SELECT database_oid::bigint,format_oid::bigint FROM gl_format WHERE singleton=true",
                    &[],
                )?;
            let current = t
                .query_one(
                    "SELECT (SELECT oid::bigint FROM pg_database WHERE datname=current_database()),'public.gl_format'::regclass::oid::bigint",
                    &[],
                )?;
            let database_oid: i64 = current.get(0usize)?;
            let format_oid: i64 = current.get(1usize)?;
            if stored.get::<_, i64>(0usize)? != database_oid
                || stored.get::<_, i64>(1usize)? != format_oid
            {
                t.rebind_restored_sources()?;
                t.execute(
                    "UPDATE gl_format SET database_oid=$1::bigint::oid,format_oid=$2::bigint::oid WHERE singleton=true",
                    &[&database_oid,&format_oid],
                )?;
            }
        }
        t.commit()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        if self.transaction {
            match &mut self.backend {
                Backend::Sqlite(c) => {
                    if let Ok(c) = c.read() {
                        let _ = c.progress_handler(0, None::<fn() -> bool>);
                        let _ = c.execute_batch("ROLLBACK");
                    }
                }
                Backend::Postgis(c) => {
                    c.invalidate();
                }
            }
        }
    }
}
#[derive(Default)]
struct TransactionState {
    datasets: HashMap<(String, String), Row>,
    heads: HashMap<String, i64>,
    verified_sources: HashSet<(String, String)>,
    stage: Vec<(String, String, Option<String>, Option<String>)>,
    stage_bytes: usize,
    custom_spatial_plan: bool,
}
pub(crate) struct SqlTransaction(Client, TransactionState);
impl SqlTransaction {
    pub(crate) fn query(&mut self, s: &str, p: &[&dyn Parameter]) -> Result<Vec<Row>> {
        self.0.query(s, p)
    }
    pub(crate) fn query_one(&mut self, s: &str, p: &[&dyn Parameter]) -> Result<Row> {
        self.0.query_one(s, p)
    }
    pub(crate) fn query_opt(&mut self, s: &str, p: &[&dyn Parameter]) -> Result<Option<Row>> {
        self.0.query_opt(s, p)
    }
    pub(crate) fn execute(&mut self, s: &str, p: &[&dyn Parameter]) -> Result<()> {
        self.0.execute(s, p)
    }
    pub(crate) fn batch_execute(&mut self, s: &str) -> Result<()> {
        self.0.batch_execute(s)
    }
    pub(crate) fn lock_sql(&self, sql: &str, lock: &str) -> String {
        match self.0.backend {
            Backend::Sqlite(_) => sql.to_owned(),
            Backend::Postgis(_) => format!("{sql} {lock}"),
        }
    }
    /// Native columns are populated on every write; raw JSON remains the immutable snapshot.
    pub(crate) fn geometry_columns(&self) -> &'static str {
        match self.0.backend {
            Backend::Sqlite(_) => "geom,geom_z",
            Backend::Postgis(_) => "geom",
        }
    }
    // Dataset CRS owns interpretation; derived indexes compare only native XY coordinates.
    pub(crate) fn geometry_values(&self, source: &str) -> String {
        match self.0.backend {
            Backend::Postgis(_) => format!("ST_SetSRID(ST_GeomFromGeoJSON({source}),0)"),
            Backend::Sqlite(_) => {
                let g = format!(
                    "coalesce(SetSRID(GeomFromGeoJSON({source}),0),SetSRID(GeomFromGeoJSON(gl_spatial_collection({source})),0),gl_check_empty_geometry({source}))"
                );
                format!(
                    "CASE WHEN CoordDimension({g})='XY' THEN {g} END,CASE WHEN CoordDimension({g})='XYZ' THEN {g} END"
                )
            }
        }
    }
    pub(crate) fn commit(mut self) -> Result<()> {
        self.0.batch_execute("COMMIT")?;
        self.0.transaction = false;
        Ok(())
    }
}
fn sqlite_error(source: rusqlite::Error) -> Error {
    use rusqlite::ErrorCode;
    if source.sqlite_error().is_some_and(|e| {
        matches!(
            e.extended_code,
            rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
        )
    }) {
        return Error::new(409, "resource already exists").caused_by(source);
    }
    let status = match source.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => 429,
        Some(ErrorCode::OperationInterrupted) => 504,
        _ => 503,
    };
    Error::new(status, "storage operation failed").caused_by(source)
}

mod authentication;
impl ResponseBudget for SqlTransaction {
    fn set_response_limit(&mut self, bytes: usize) {
        self.0.result_limit = bytes;
        if let Backend::Postgis(c) = &mut self.0.backend {
            c.set_result_limit(bytes);
        }
    }

    fn response_limit(&self) -> usize {
        self.0.result_limit
    }

    fn set_response_reservation(&mut self, reservation: Option<crate::ResponseReservation>) {
        self.0.response_reservation = reservation.clone();
        if let Backend::Postgis(c) = &mut self.0.backend {
            c.set_response_reservation(reservation);
        }
    }

    fn response_reservation(&self) -> Option<crate::ResponseReservation> {
        self.0.response_reservation.clone()
    }
}

impl RepositoryTransaction for SqlTransaction {
    fn commit(self: Box<Self>) -> Result<()> {
        SqlTransaction::commit(*self)
    }
}

/// SQL NULL accepted for any parameter type (a typed `None` only matches its own type).
#[derive(Debug)]
struct AnyNull;
impl tokio_postgres::types::ToSql for AnyNull {
    fn to_sql(
        &self,
        _: &tokio_postgres::types::Type,
        _: &mut bytes::BytesMut,
    ) -> std::result::Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        Ok(tokio_postgres::types::IsNull::Yes)
    }
    fn accepts(_: &tokio_postgres::types::Type) -> bool {
        true
    }
    tokio_postgres::types::to_sql_checked!();
}
pub(crate) use backend::SqlStorage;

/// Check that a SQLite file is an intact GeoLedger database at the current format.
pub(crate) fn verify_sqlite_file(path: &std::path::Path, timeout: Duration) -> Result<()> {
    if !path.is_file() {
        return Err(Error::new(404, "database file not found"));
    }
    let mut c = Client::open(
        &Storage::Sqlite(path.to_owned()),
        &Arc::new(postgres::Pool::default()),
        timeout,
    )?;
    c.batch_execute("PRAGMA query_only=ON")?;
    let check = c
        .query_one("PRAGMA integrity_check", &[])?
        .get::<_, String>(0usize)?;
    if check != "ok" {
        return Err(Error::new(409, "SQLite integrity check failed"));
    }
    let exists = c
        .query_opt(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_format'",
            &[],
        )?
        .is_some();
    if !exists {
        return Err(Error::new(409, "not a GeoLedger database"));
    }
    let version = c
        .query_one("SELECT version FROM gl_format WHERE singleton=true", &[])?
        .get::<_, i32>(0usize)?;
    if version != FORMAT_VERSION {
        return Err(Error::new(
            409,
            &format!("database has storage format {version}; this server uses {FORMAT_VERSION}"),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test fixtures and assertions should fail immediately"
)]
mod tests {
    use super::*;

    fn seed_current_from_history(
        t: &mut SqlTransaction,
        project: &str,
        dataset: &str,
    ) -> Result<()> {
        t.create_dataset_table(project, dataset)?;
        let table = t
            .dataset_table(project, dataset)?
            .ok_or_else(|| Error::new(500, "missing fixture table"))?;
        let columns = t.geometry_columns();
        if matches!(t.0.backend, Backend::Postgis(_)) {
            t.query_one("SELECT set_config('geoledger.publication_table',($1::text::regclass::oid)::text,true)", &[&table])?;
        }
        t.with_dataset_writes(|t| t.execute(&format!("INSERT INTO {table}(feature_id,valid_from,properties,geometry_json,{columns}) SELECT feature_id,valid_from,properties,geometry_json,{columns} FROM gl_history WHERE project=$1 AND dataset=$2 AND valid_to IS NULL AND properties IS NOT NULL"), &[&project,&dataset]))?;
        if matches!(t.0.backend, Backend::Postgis(_)) {
            t.batch_execute("SET LOCAL geoledger.publication_table = ''")?;
        }
        t.execute("INSERT INTO gl_purge VALUES($1)", &[&project])?;
        t.execute(
            "DELETE FROM gl_history WHERE project=$1 AND dataset=$2 AND valid_to IS NULL",
            &[&project, &dataset],
        )?;
        t.execute("DELETE FROM gl_purge WHERE project=$1", &[&project])
    }

    fn capability_atomicity(storage: Storage) -> Result<()> {
        let timeout = Duration::from_secs(30);
        let backend = SqlStorage {
            storage,
            pool: Arc::new(postgres::Pool::default()),
        };
        backend.initialize(timeout)?;
        let project = uuid::Uuid::new_v4().to_string();
        let dataset = uuid::Uuid::new_v4().to_string();
        let workspace = uuid::Uuid::new_v4().to_string();
        let subject = uuid::Uuid::new_v4().to_string();
        let request = uuid::Uuid::new_v4().to_string();
        let user = crate::authentication::UserRecord {
            user: crate::User {
                id: subject.clone(),
                username: subject.clone(),
                display_name: "Atomic capabilities".into(),
                status: "active".into(),
                is_admin: false,
                must_change_password: true,
                created_at: "2026-10-21T00:00:00Z".into(),
            },
            password_hash: "unused test hash".into(),
            credential_version: 0,
        };
        for outcome in ["drop", "error", "commit"] {
            let mut owner = backend.begin(false, timeout)?;
            let credentials: &mut dyn CredentialRepository = owner.as_mut();
            credentials.lock_credentials()?;
            credentials.auth_insert_user(&user)?;
            credentials.auth_audit(&subject, "test", &subject, &user.user.created_at)?;
            let projects: &mut dyn ProjectRepository = owner.as_mut();
            projects.insert_project(&project, "Atomic capabilities")?;
            projects.insert_owner(&project, &subject)?;
            let datasets: &mut dyn DatasetRepository = owner.as_mut();
            datasets.insert_dataset(
                &project,
                &dataset,
                "Features",
                "point",
                2,
                "EPSG:4326",
                true,
            )?;
            let drafts: &mut dyn WorkspaceRepository = owner.as_mut();
            drafts.insert_workspace(&project, &workspace, &subject, 0)?;
            let publication: &mut dyn PublicationRepository = owner.as_mut();
            publication.begin_merge()?;
            publication.stage_merge(
                &dataset,
                "one",
                &None,
                &Some(r#"{"properties":{"value":1},"geometry":null}"#.into()),
            )?;
            publication.append_commit(&project, 1, &workspace, &subject, "Atomic publication")?;
            publication.append_changes(&project, 1)?;
            publication.append_history(&project, 1)?;
            publication.advance_head(&project, 1)?;
            publication.save_receipt(&project, &subject, &request, "{}", "{}")?;
            owner.advance_workspace(&project, &workspace, "published")?;
            let audit: &mut dyn AuditRepository = owner.as_mut();
            audit.append_audit(&project, &subject, "test", "{}")?;
            if outcome == "error" {
                // Fail after all capabilities wrote; the missing member violates the audit FK.
                assert!(
                    audit
                        .append_audit(&project, "missing-member", "test", "{}")
                        .is_err()
                );
            }
            if outcome == "commit" {
                owner.commit()?;
            } else {
                drop(owner);
            }
            let committed = outcome == "commit";
            let mut reader = backend.begin(true, timeout)?;
            assert_eq!(
                reader
                    .project_head(&project, false)?
                    .map(|row| row.get::<_, i64>(0usize))
                    .transpose()?,
                committed.then_some(1)
            );
            assert_eq!(
                reader.dataset_exists(&project, &dataset)?.is_some(),
                committed
            );
            assert_eq!(reader.auth_user(&subject)?.is_some(), committed);
            assert_eq!(
                reader.feature_at(&project, &dataset, "one", 1)?.is_some(),
                committed
            );
            assert_eq!(
                reader.history_page(&project, 0, 10)?.len(),
                usize::from(committed)
            );
            assert_eq!(
                reader.audit_page(&project, 0, 10)?.len(),
                usize::from(committed)
            );
            assert_eq!(
                reader
                    .publication_receipt(&project, &subject, &request)?
                    .is_some(),
                committed
            );
            assert_eq!(
                reader
                    .workspace_state(&project, &workspace, &subject, false)?
                    .map(|row| row.get::<_, String>(2usize))
                    .transpose()?,
                committed.then(|| "published".to_owned())
            );
            reader.commit()?;
            let mut monitor = Client::open(&backend.storage, &backend.pool, timeout)?;
            assert_eq!(
                monitor
                    .query_one(
                        "SELECT count(*) FROM gl_auth_audit WHERE actor=$1",
                        &[&subject]
                    )?
                    .get::<_, i64>(0usize)?,
                i64::from(committed)
            );
        }
        Ok(())
    }

    #[test]
    fn sqlite_capabilities_share_commit_error_and_drop_rollback() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        capability_atomicity(Storage::Sqlite(dir.path().join("capabilities.db")))
    }

    #[test]
    #[ignore = "requires isolated geoledger_test PostGIS database"]
    fn postgis_capabilities_share_commit_error_and_drop_rollback() -> Result<()> {
        let dsn = std::env::var("GL_TEST_DATABASE_URL").unwrap();
        let pool = Arc::new(postgres::Pool::default());
        let mut monitor = Client::open(
            &Storage::Postgis(dsn.clone()),
            &pool,
            Duration::from_secs(30),
        )?;
        assert_eq!(
            monitor
                .query_one("SELECT current_database()::text", &[])?
                .get::<_, String>(0usize)?,
            "geoledger_test"
        );
        let result = capability_atomicity(Storage::Postgis(dsn));
        // Later SDK tests need an empty identity store to bootstrap their administrator.
        monitor.batch_execute(
            "DROP SCHEMA IF EXISTS business CASCADE; DROP SCHEMA public CASCADE; CREATE SCHEMA public; CREATE EXTENSION IF NOT EXISTS postgis;",
        )?;
        result
    }

    #[test]
    fn sqlite_json_field_preserves_raw_json_and_unquotes_strings() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let mut client = Client::open(
            &Storage::Sqlite(dir.path().join("json.db")),
            &Arc::new(postgres::Pool::default()),
            // This is a JSON correctness test, not a latency assertion. Native
            // initialization can queue behind the other parallel storage tests.
            Duration::from_secs(30),
        )?;
        for (source, key, expected) in [
            (
                r#"{"properties":{"n":1.2300,"nested":{"$serde_json::private::Number":"123"}}}"#,
                "properties",
                Some(r#"{"n":1.2300,"nested":{"$serde_json::private::Number":"123"}}"#),
            ),
            (
                r#"{"geometry":"{\"type\":\"Point\",\"coordinates\":[1,2]}"}"#,
                "geometry",
                Some(r#"{"type":"Point","coordinates":[1,2]}"#),
            ),
            (r#"{"geometry":null}"#, "geometry", None),
            (r#"{}"#, "missing", None),
            ("null", "properties", None),
        ] {
            assert_eq!(
                client
                    .query_one("SELECT gl_json_field($1,$2)", &[&source, &key])?
                    .get::<_, Option<String>>(0usize)?
                    .as_deref(),
                expected
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "requires isolated geoledger_test PostGIS database"]
    fn postgis_audit_cursor_follows_commit_order_without_blocking_other_projects() -> Result<()> {
        let storage = Storage::Postgis(std::env::var("GL_TEST_DATABASE_URL").unwrap());
        let pool = Arc::new(postgres::Pool::default());
        let timeout = Duration::from_secs(10);
        let mut monitor = Client::open(&storage, &pool, timeout)?;
        assert_eq!(
            monitor
                .query_one("SELECT current_database()::text", &[])?
                .get::<_, String>(0usize)?,
            "geoledger_test"
        );
        Client::open(&storage, &pool, timeout)?.migrate()?;
        let project = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let mut setup = Client::open(&storage, &pool, timeout)?.transaction(false)?;
        for p in [&project, &other] {
            setup.insert_project(p, "audit ordering")?;
            setup.insert_owner(p, "alice")?;
        }
        setup.commit()?;
        let mut first = Client::open(&storage, &pool, timeout)?.transaction(false)?;
        first.append_audit(&project, "alice", "first", "{}")?;
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (appended_tx, appended_rx) = std::sync::mpsc::channel();
        let (commit_tx, commit_rx) = std::sync::mpsc::channel();
        let worker_storage = storage.clone();
        let worker_pool = pool.clone();
        let worker_project = project.clone();
        let worker = std::thread::spawn(move || -> Result<()> {
            let mut second =
                Client::open(&worker_storage, &worker_pool, timeout)?.transaction(false)?;
            let pid = second
                .query_one("SELECT pg_backend_pid()", &[])?
                .get::<_, i32>(0usize)?;
            started_tx.send(pid).unwrap();
            second.append_audit(&worker_project, "alice", "second", "{}")?;
            appended_tx.send(()).unwrap();
            commit_rx.recv().unwrap();
            second.commit()
        });
        let pid = started_rx.recv_timeout(timeout).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let blocked = loop {
            if monitor
                .query_one(
                    "SELECT cardinality(pg_blocking_pids($1::bigint::integer)) > 0",
                    &[&pid],
                )?
                .get::<_, bool>(0usize)?
            {
                break true;
            }
            if appended_rx.try_recv().is_ok() || Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut independent = Client::open(&storage, &pool, timeout)?.transaction(false)?;
        independent.append_audit(&other, "alice", "independent", "{}")?;
        independent.commit()?;
        first.commit()?;
        if !blocked {
            commit_tx.send(()).unwrap();
            worker.join().unwrap()?;
            panic!("same-project audit allocation overtook an uncommitted audit event");
        }
        appended_rx.recv_timeout(timeout).unwrap();
        let mut reader = Client::open(&storage, &pool, timeout)?.transaction(true)?;
        let page = reader.audit_page(&project, 0, 10)?;
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].get::<_, String>(2usize)?, "first");
        let after = page[0].get::<_, i64>(0usize)?;
        assert!(!reader.audit_has_after(&project, after)?);
        commit_tx.send(()).unwrap();
        worker.join().unwrap()?;
        // A commit between the page read and its continuation probe cannot
        // change the end signal within this REPEATABLE READ transaction.
        assert!(!reader.audit_has_after(&project, after)?);
        assert!(reader.audit_page(&project, after, 10)?.is_empty());
        reader.commit()?;
        let mut reader = Client::open(&storage, &pool, timeout)?.transaction(true)?;
        assert!(reader.audit_has_after(&project, after)?);
        let page = reader.audit_page(&project, after, 10)?;
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].get::<_, String>(2usize)?, "second");
        Ok(())
    }

    mod query_performance_tests {
        #![allow(
            clippy::unwrap_used,
            reason = "test fixtures and assertions should fail immediately"
        )]
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[test]
        fn sqlite_empty_bbox_and_invalidation_do_not_scan_unrelated_drafts() -> Result<()> {
            let dir = tempfile::tempdir().unwrap();
            let storage = Storage::Sqlite(dir.path().join("query.db"));
            let pool = Arc::new(postgres::Pool::default());
            Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
            let mut t =
                Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
            t.batch_execute(
        r#"INSERT INTO gl_projects(id,name) VALUES('p','p');
        INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner');
        INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','d','point');
        INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0);
        INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('p',1,'w','alice','seed');
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,geom)
        VALUES('p','d','seed',1,'{}','{"type":"Point","coordinates":[20,20]}',SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[20,20]}'),0));
        ANALYZE gl_history;
        INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties) VALUES('p','w','d','seed','{}');
        ANALYZE gl_workspace_changes;
        WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<2000)
        INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties)
        SELECT 'p','w','d',printf('%08d',n),'{}' FROM ids;
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties)
        SELECT project,dataset,feature_id,1,properties FROM gl_workspace_changes WHERE feature_id<>'seed';
        UPDATE gl_workspace_changes SET resolved_head=0 WHERE feature_id='00000001';"#,
    )?;
            seed_current_from_history(&mut t, "p", "d")?;
            let steps = Arc::new(AtomicUsize::new(0));
            let observed = steps.clone();
            if let Backend::Sqlite(c) = &t.0.backend {
                let c = c.read().map_err(sqlite_error)?;
                c.progress_handler(
                    1,
                    Some(move || {
                        observed.fetch_add(1, Ordering::Relaxed);
                        false
                    }),
                )
                .map_err(sqlite_error)?;
            }
            assert!(
                !t.edit_base("p", "w", "d", "absent", 1)?
                    .get::<_, bool>(0usize)?
            );
            let lookup_steps = steps.swap(0, Ordering::Relaxed);
            assert!(
                lookup_steps < 1000,
                "stale ANALYZE caused resolution lookup scan: {lookup_steps}"
            );
            t.batch_execute(r#"WITH RECURSIVE ids(n) AS (SELECT 2 UNION ALL SELECT n+1 FROM ids WHERE n<1000)
        INSERT INTO gl_commits(project,revision,workspace,subject,message) SELECT 'p',n,'w','alice','history' FROM ids;
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties)
        SELECT 'p','d','versions',revision,CASE WHEN revision<1000 THEN revision+1 END,'{"v":'||revision||'}'
        FROM gl_commits WHERE project='p';"#)?;
            steps.store(0, Ordering::Relaxed);
            let historical = t.edit_base("p", "w", "d", "versions", 500)?;
            assert_eq!(historical.get::<_, String>(1usize)?, r#"{"v":500}"#);
            assert!(
                steps.swap(0, Ordering::Relaxed) < 1000,
                "base lookup scanned earlier versions"
            );
            t.remove_delta("p", "w", "d", "absent")?;
            let delete_steps = steps.swap(0, Ordering::Relaxed);
            assert!(
                delete_steps < 1000,
                "stale ANALYZE caused delta deletion scan: {delete_steps}"
            );
            t.clear_deltas("p", "absent")?;
            let clear_steps = steps.swap(0, Ordering::Relaxed);
            assert!(
                clear_steps < 1000,
                "stale ANALYZE caused workspace deletion scan: {clear_steps}"
            );
            t.invalidate_resolutions("p", "w")?;
            let invalidation_steps = steps.swap(0, Ordering::Relaxed);
            assert!(
                invalidation_steps < 1000,
                "invalidation scanned draft rows: {invalidation_steps}"
            );
            assert_eq!(
                t.query_one("SELECT changes()", &[])?
                    .get::<_, i64>(0usize)?,
                1
            );
            t.invalidate_resolutions("p", "w")?;
            assert_eq!(
                t.query_one("SELECT changes()", &[])?
                    .get::<_, i64>(0usize)?,
                0
            );
            steps.store(0, Ordering::Relaxed);
            let rows = t.feature_page(&crate::repository::FeatureQuery {
                project: "p".into(),
                dataset: "d".into(),
                revision: 1,
                workspace: Some("w".into()),
                after: String::new(),
                feature_id: None,
                bbox: Some([-1., -1., 1., 1.]),
                limit: 32,
            })?;
            let query_steps = steps.load(Ordering::Relaxed);
            assert!(rows.is_empty());
            assert!(
                query_steps < 1000,
                "empty Rtree query scanned draft rows: {query_steps}"
            );
            t.batch_execute(r#"UPDATE gl_workspace_changes SET geom=SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[0,0]}'),0),geometry_json='{"type":"Point","coordinates":[0,0]}'"#)?;
            let dense = crate::repository::FeatureQuery {
                project: "p".into(),
                dataset: "d".into(),
                revision: 1,
                workspace: Some("w".into()),
                after: String::new(),
                feature_id: None,
                bbox: Some([-180., -90., 180., 90.]),
                limit: 20,
            };
            assert!(!t.sparse_spatial_candidates("workspace_changes", &dense)?);
            steps.store(0, Ordering::Relaxed);
            let rows = t.feature_page(&dense)?;
            let dense_steps = steps.load(Ordering::Relaxed);
            assert_eq!(rows.len(), 20);
            assert_eq!(rows[0].get::<_, String>(0usize)?, "00000001");
            assert!(
                dense_steps < 5000,
                "dense bbox enumerated all candidates: {dense_steps}"
            );
            t.batch_execute(r#"UPDATE gl_workspace_changes SET geom=SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[20,20]}'),0),geom_z=SetSRID(GeomFromGeoJSON('{"type":"Point","coordinates":[20,20,3]}'),0) WHERE feature_id='00002000'"#)?;
            let sparse = crate::repository::FeatureQuery {
                bbox: Some([19., 19., 21., 21.]),
                ..dense
            };
            steps.store(0, Ordering::Relaxed);
            let rows = t.feature_page(&sparse)?;
            let sparse_steps = steps.load(Ordering::Relaxed);
            assert_eq!(
                rows.len(),
                1,
                "XY/XYZ candidates for one row must be deduplicated"
            );
            assert_eq!(rows[0].get::<_, String>(0usize)?, "00002000");
            assert!(
                sparse_steps < 1000,
                "stale ANALYZE caused sparse candidate scan: {sparse_steps}"
            );
            let historical = crate::repository::FeatureQuery {
                workspace: None,
                ..sparse
            };
            steps.store(0, Ordering::Relaxed);
            let rows = t.feature_page(&historical)?;
            let history_steps = steps.load(Ordering::Relaxed);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get::<_, String>(0usize)?, "seed");
            assert!(
                history_steps < 1000,
                "stale ANALYZE caused history candidate scan: {history_steps}"
            );
            Ok(())
        }

        #[test]
        fn sqlite_sparse_bbox_above_page_cap_uses_candidates_in_scoped_history_and_drafts()
        -> Result<()> {
            let dir = tempfile::tempdir().unwrap();
            let storage = Storage::Sqlite(dir.path().join("sparse.db"));
            let pool = Arc::new(postgres::Pool::default());
            Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
            let mut t =
                Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
            t.batch_execute(r#"
        INSERT INTO gl_projects(id,name) VALUES('p','p');
        INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner');
        INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','d','point'),('p','other','other','point');
        INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0);
        INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('p',1,'w','alice','seed'),('p',2,'w','alice','later');
        WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<50000)
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,geom)
        SELECT 'p','d',printf('%08d',n),1,'{}','{"type":"Point","coordinates":[20,20]}',MakePoint(CASE WHEN n>49900 THEN 0 ELSE 20 END,CASE WHEN n>49900 THEN 0 ELSE 20 END,0) FROM ids;
        INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geom)
        SELECT project,'w',dataset,feature_id,properties,geom FROM gl_history;
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geom)
        SELECT project,'other',feature_id,1,properties,MakePoint(0,0,0) FROM gl_history WHERE feature_id<='00000100';
        UPDATE gl_history SET valid_to=2 WHERE dataset='d' AND feature_id<='00000100';
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geom)
        SELECT project,dataset,feature_id,2,properties,MakePoint(0,0,0) FROM gl_history WHERE dataset='d' AND feature_id<='00000100';
        INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geom)
        VALUES ('p','d','00050001',1,'{}',MakePoint(40,40,0)),('p','d','00050002',1,'{}',MakePoint(40,40,0)),('p','d','00050003',1,'{}',MakePoint(40,40,0));
        INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geom)
        SELECT project,'w',dataset,feature_id,properties,geom FROM gl_history WHERE dataset='d' AND feature_id>'00050000';
    "#)?;
            seed_current_from_history(&mut t, "p", "d")?;
            seed_current_from_history(&mut t, "p", "other")?;
            let steps = Arc::new(AtomicUsize::new(0));
            let observed = steps.clone();
            if let Backend::Sqlite(c) = &t.0.backend {
                let c = c.read().map_err(sqlite_error)?;
                c.progress_handler(
                    1,
                    Some(move || {
                        observed.fetch_add(1, Ordering::Relaxed);
                        false
                    }),
                )
                .map_err(sqlite_error)?;
            }
            for workspace in [None, Some("w".to_owned())] {
                let query = crate::repository::FeatureQuery {
                    project: "p".into(),
                    dataset: "d".into(),
                    revision: 1,
                    workspace,
                    after: String::new(),
                    feature_id: None,
                    bbox: Some([-1., -1., 1., 1.]),
                    limit: 20,
                };
                steps.store(0, Ordering::Relaxed);
                let rows = t.feature_page(&query)?;
                let query_steps = steps.load(Ordering::Relaxed);
                assert_eq!(rows.len(), 20);
                assert_eq!(rows[0].get::<_, String>(0usize)?, "00049901");
                assert!(
                    query_steps < 30000,
                    "sparse bbox scanned background: {query_steps}"
                );
                steps.store(0, Ordering::Relaxed);
                assert!(t.feature_exists(&query)?);
                let probe_steps = steps.load(Ordering::Relaxed);
                assert!(
                    probe_steps < 30000,
                    "feature lookahead scanned background: {probe_steps}"
                );
                for (after, bbox) in [
                    ("00050003", [-180., -90., 180., 90.]),
                    ("00050000", [-30., -30., 30., 30.]),
                ] {
                    let exhausted = crate::repository::FeatureQuery {
                        after: after.into(),
                        bbox: Some(bbox),
                        ..query.clone()
                    };
                    steps.store(0, Ordering::Relaxed);
                    assert!(!t.feature_exists(&exhausted)?);
                    let exhausted_steps = steps.load(Ordering::Relaxed);
                    assert!(
                        exhausted_steps < 30000,
                        "terminal feature lookahead scanned background: {exhausted_steps}"
                    );
                }
                let next = crate::repository::FeatureQuery {
                    after: "00049980".into(),
                    ..query
                };
                let rows = t.feature_page(&next)?;
                assert_eq!(rows.len(), 20);
                assert_eq!(rows[0].get::<_, String>(0usize)?, "00049981");
            }
            Ok(())
        }

        #[test]
        fn sqlite_publication_closes_only_changed_history_keys() -> Result<()> {
            let dir = tempfile::tempdir().unwrap();
            let storage = Storage::Sqlite(dir.path().join("close-history.db"));
            let pool = Arc::new(postgres::Pool::default());
            Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
            let mut t =
                Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
            t.batch_execute("INSERT INTO gl_projects(id,name) VALUES('p','p'); INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0); INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('p',1,'w','alice','seed'),('p',2,'w','alice','edit'); WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<5000) INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties) SELECT 'p','d',printf('%08d',n),1,'{}' FROM ids;")?;
            seed_current_from_history(&mut t, "p", "d")?;
            t.begin_merge()?;
            t.stage_merge("d", "00001000", &Some("{}".into()), &None)?;
            t.flush_stage()?;
            let steps = Arc::new(AtomicUsize::new(0));
            let observed = steps.clone();
            if let Backend::Sqlite(c) = &t.0.backend {
                c.read()
                    .map_err(sqlite_error)?
                    .progress_handler(
                        1,
                        Some(move || {
                            observed.fetch_add(1, Ordering::Relaxed);
                            false
                        }),
                    )
                    .map_err(sqlite_error)?;
            }
            t.close_history("p", 2)?;
            let work = steps.load(Ordering::Relaxed);
            assert!(
                work < 1000,
                "closing one feature scanned unrelated history: {work}"
            );
            assert_eq!(
                t.query_one("SELECT count(*) FROM gl_history WHERE valid_to=2", &[])?
                    .get::<_, i64>(0usize)?,
                1
            );
            let table = t
                .dataset_table("p", "d")?
                .ok_or_else(|| Error::new(500, "fixture table"))?;
            assert_eq!(
                t.query_one(
                    &format!("SELECT count(*) FROM {table} WHERE feature_id='00001001'"),
                    &[]
                )?
                .get::<_, i64>(0usize)?,
                1
            );
            Ok(())
        }
    }

    mod review_tests {
        //! Measured work bounds for the review fixes, not wall-clock throughput claims.
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};
        type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

        #[test]
        fn sqlite_count_metadata_and_stage_work_stays_bounded() -> TestResult {
            let dir = tempfile::tempdir()?;
            let storage = Storage::Sqlite(dir.path().join("bounded.db"));
            let pool = Arc::new(postgres::Pool::default());
            Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
            let mut t =
                Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
            t.batch_execute("INSERT INTO gl_projects(id,name) VALUES('p','p'); INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0);")?;
            let steps = Arc::new(AtomicUsize::new(0));
            let observed = steps.clone();
            if let Backend::Sqlite(c) = &t.0.backend {
                let c = c.read().map_err(sqlite_error)?;
                c.progress_handler(
                    1,
                    Some(move || {
                        observed.fetch_add(1, Ordering::Relaxed);
                        false
                    }),
                )?;
            }
            for target in [1000_i64, 10000, 100000] {
                let have: i64 = t.count_deltas("p", "w")?.get(0usize)?;
                t.execute("WITH RECURSIVE ids(n) AS (SELECT $1 UNION ALL SELECT n+1 FROM ids WHERE n<$2) INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties) SELECT 'p','w','d',printf('%08d',n),'{}' FROM ids", &[&(have+1),&target])?;
                steps.store(0, Ordering::Relaxed);
                assert_eq!(t.count_deltas("p", "w")?.get::<_, i64>(0usize)?, target);
                let count_steps = steps.swap(0, Ordering::Relaxed);
                assert!(
                    count_steps < 100,
                    "count scanned {target} drafts: {count_steps}"
                );
                eprintln!("derived_count drafts={target} vm_steps={count_steps}");
            }
            t.dataset_exists("p", "d")?.ok_or("dataset")?;
            steps.store(0, Ordering::Relaxed);
            for _ in 0..1000 {
                t.dataset_exists("p", "d")?.ok_or("dataset")?;
            }
            assert_eq!(
                steps.load(Ordering::Relaxed),
                0,
                "metadata cache re-queried database"
            );
            t.begin_merge()?;
            for n in 0..100 {
                t.stage_merge("d", &n.to_string(), &None, &Some("{}".into()))?;
            }
            let staged: i64 = t
                .query_one("SELECT count(*) FROM gl_merge_stage", &[])?
                .get(0usize)?;
            assert_eq!(
                staged, 96,
                "three full 32-row batches must already be flushed"
            );
            assert_eq!(t.1.stage.len(), 4);
            t.flush_stage()?;
            assert_eq!(
                t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
                    .get::<_, i64>(0usize)?,
                100
            );
            eprintln!(
                "metadata_calls=1000 additional_vm_steps=0; staged_rows=100 full_batches=3 final_batch=4"
            );
            Ok(())
        }

        #[test]
        fn sqlite_conflict_cache_reuses_analysis_and_invalidates_on_version() -> TestResult {
            use serde_json::{Value, json};
            let dir = tempfile::tempdir()?;
            let path = dir.path().join("cache.db");
            let app = crate::Application::new(Storage::Sqlite(path.clone()));
            app.migrate()?;
            let p = uuid::Uuid::new_v4().to_string();
            let d = uuid::Uuid::new_v4().to_string();
            let w = uuid::Uuid::new_v4().to_string();
            let db = rusqlite::Connection::open(&path)?;
            let db = geoledger_spatialite::load_configured(db)?;
            db.read()?.execute_batch(&format!("PRAGMA foreign_keys=ON; BEGIN; INSERT INTO gl_projects(id,name,head) VALUES('{p}','cache',2); INSERT INTO gl_project_members(project,subject,role) VALUES('{p}','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('{p}','{d}','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('{p}','{w}','alice',1); INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('{p}',1,'{w}','alice','base'),('{p}',2,'{w}','alice','current'); WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<128) INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties) SELECT '{p}','{d}',printf('%08d',n),1,2,'{{\"a\":0}}' FROM ids; INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties) SELECT project,dataset,feature_id,2,'{{\"a\":1}}' FROM gl_history WHERE project='{p}'; INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties) SELECT project,'{w}',dataset,feature_id,'{{\"a\":2}}' FROM gl_history WHERE project='{p}' AND valid_from=1; COMMIT;"))?;
            let mut fixture = Client::open(
                &Storage::Sqlite(path.clone()),
                &Arc::new(postgres::Pool::default()),
                Duration::from_secs(30),
            )?
            .transaction(false)?;
            seed_current_from_history(&mut fixture, &p, &d)?;
            fixture.commit()?;
            let query = |after: &str| {
                app.execute(
                    "alice",
                    "conflicts",
                    json!({"project":p,"workspace":w,"after":after,"limit":10}),
                )
            };
            assert_eq!(query("")?["total"], 128);
            db.read()?.execute_batch("CREATE TRIGGER reject_cache_rebuild BEFORE DELETE ON gl_conflict_cache BEGIN SELECT RAISE(ABORT,'unchanged cache was rebuilt'); END;")?;
            let page = query(&format!("{d}/00000100"))?;
            assert_eq!(page["conflicts"].as_array().ok_or("conflicts")?.len(), 10);
            assert_eq!(page["total"], 128);
            assert!(
                query(&format!("{d}/00000128"))?["conflicts"]
                    .as_array()
                    .ok_or("conflicts")?
                    .is_empty()
            );
            db.read()?
                .execute_batch("DROP TRIGGER reject_cache_rebuild")?;
            app.execute("alice","save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"00000001","feature":{"type":"Feature","id":"00000001","properties":{"a":1},"geometry":Value::Null}}]}))?;
            assert_eq!(query("")?["total"], 127);
            let size: i64 =
                db.read()?
                    .query_row("SELECT count(*) FROM gl_conflict_cache", [], |row| {
                        row.get(0)
                    })?;
            assert_eq!(size, 1, "old cache generations accumulated");
            eprintln!(
                "conflicts=128 cached_late_page=10 cached_empty_page=0 recomputations=0; changed_version_conflicts=127 cache_generations=1"
            );
            Ok(())
        }

        #[test]
        #[ignore = "requires isolated geoledger_test PostGIS database"]
        fn postgis_hot_spatial_queries_and_current_history_use_bounded_plans() -> TestResult {
            let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
            let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
            let name: String = admin.query_one("SELECT current_database()", &[])?.get(0);
            if name != "geoledger_test" {
                return Err("requires isolated geoledger_test".into());
            }
            let storage = Storage::Postgis(dsn);
            let pool = Arc::new(postgres::Pool::default());
            let timeout = Duration::from_secs(30);
            Client::open(&storage, &pool, timeout)?.migrate()?;
            let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
            let p = uuid::Uuid::new_v4().to_string();
            let d = uuid::Uuid::new_v4().to_string();
            let w = uuid::Uuid::new_v4().to_string();
            t.batch_execute(&format!("INSERT INTO gl_projects(id,name,head) VALUES('{p}','history plans',1000); INSERT INTO gl_project_members(project,subject,role) VALUES('{p}','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('{p}','{d}','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('{p}','{w}','alice',0); INSERT INTO gl_commits(project,revision,workspace,subject,message) SELECT '{p}',n,'{w}','alice','version' FROM generate_series(1,1000)n; INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties,geometry_json,geom) SELECT '{p}','{d}',lpad(k::text,8,'0'),v,CASE WHEN v<1000 THEN v+1 END,'{{\"v\":'||v::text||'}}','{{\"type\":\"Point\",\"coordinates\":[0,0]}}',ST_SetSRID(ST_MakePoint(0,0),0) FROM generate_series(1,64)k CROSS JOIN generate_series(1,1000)v; ANALYZE gl_history;"))?;
            seed_current_from_history(&mut t, &p, &d)?;
            t.project_head(&p, false)?.ok_or("project")?;
            let query = crate::repository::FeatureQuery {
                project: p.clone(),
                dataset: d.clone(),
                revision: 1000,
                workspace: None,
                after: String::new(),
                feature_id: None,
                bbox: Some([-180., -90., 180., 90.]),
                limit: 32,
            };
            for _ in 0..10 {
                assert_eq!(t.feature_page(&query)?.len(), 32);
            }
            let statements=t.query("SELECT name,generic_plans,custom_plans FROM pg_prepared_statements WHERE statement LIKE 'WITH current AS MATERIALIZED%'",&[])?;
            let statement = statements.first().ok_or("cached spatial statement")?;
            assert_eq!(
                statement.get::<_, i64>(1usize)?,
                0,
                "generic bbox plan was reused"
            );
            assert!(statement.get::<_, i64>(2usize)? >= 10);
            let name: String = statement.get(0usize)?;
            let rows=t.query(&format!("EXPLAIN (ANALYZE, BUFFERS, COSTS OFF) EXECUTE \"{}\" ('{p}','{d}',1000,NULL,'',NULL,true,-180,-90,180,90,32)",name.replace('"',"\"\"")),&[])?;
            let plan = rows
                .iter()
                .map(|row| row.get::<_, String>(0usize))
                .collect::<Result<Vec<_>>>()?
                .join("\n");
            assert!(
                !plan.contains("gl_history"),
                "current-head query scanned historical bodies: {plan}"
            );
            assert!(
                !plan.contains("Rows Removed by Filter: 31968"),
                "old versions still scanned: {plan}"
            );
            eprintln!(
                "hot_bbox_executions=10 generic_plans=0 current_rows=64 historical_versions_each=999 page_rows=32\n{plan}"
            );
            let historic = crate::repository::FeatureQuery {
                revision: 1,
                bbox: None,
                ..query
            };
            let rows = t.feature_page(&historic)?;
            assert_eq!(rows.len(), 32);
            let properties: serde_json::Value =
                serde_json::from_str(&rows[0].get::<_, String>(1usize)?)?;
            assert_eq!(
                properties["v"], 1,
                "latest optimization broke historical reads"
            );
            Ok(())
        }

        #[test]
        #[ignore = "requires isolated geoledger_test PostGIS database"]
        fn postgis_hot_spatial_queries_merge_stage_lookup_is_indexed() -> TestResult {
            let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
            let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
            let database: String = admin.query_one("SELECT current_database()", &[])?.get(0);
            if database != "geoledger_test" {
                return Err("requires isolated geoledger_test".into());
            }
            let storage = Storage::Postgis(dsn);
            let pool = Arc::new(postgres::Pool::default());
            let mut t =
                Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
            t.begin_merge()?;
            for n in 0..500 {
                t.stage_merge("d", &n.to_string(), &None, &Some(n.to_string()))?;
            }
            t.flush_stage()?;
            // Fresh tracking tables can estimate one key; verification must not scan the stage per key.
            t.batch_execute("CREATE TEMP TABLE tracked_keys(schema_name text,table_name text,feature_id text,PRIMARY KEY(schema_name,table_name,feature_id)); INSERT INTO tracked_keys SELECT 'business','source',feature_id FROM gl_merge_stage")?;
            let rows = t.query("EXPLAIN (ANALYZE, COSTS OFF) SELECT EXISTS(SELECT 1 FROM tracked_keys k LEFT JOIN gl_merge_stage s ON s.dataset='d' AND s.feature_id=k.feature_id WHERE k.schema_name='business' AND k.table_name='source' AND s.after_value IS DISTINCT FROM k.feature_id)", &[])?;
            let plan = rows
                .iter()
                .map(|r| r.get::<_, String>(0usize))
                .collect::<Result<Vec<_>>>()?
                .join("\n");
            assert!(
                plan.contains("gl_merge_stage_pkey"),
                "stage lookup lost index: {plan}"
            );
            assert!(
                !plan.contains("Rows Removed by Join Filter: 249500"),
                "quadratic stage scan: {plan}"
            );
            eprintln!("{plan}");
            Ok(())
        }

        #[test]
        #[ignore = "requires isolated geoledger_test PostGIS database"]
        fn postgis_hot_spatial_queries_pipeline_preserves_statement_budgets() -> TestResult {
            let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
            let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
            let name: String = admin.query_one("SELECT current_database()", &[])?.get(0);
            assert_eq!(name, "geoledger_test");
            let pool = Arc::new(postgres::Pool::new(crate::StorageOptions {
                statement_timeout: Duration::from_millis(80),
                lock_timeout: Duration::from_millis(40),
                ..crate::StorageOptions::default()
            }));
            for batch in [false, true] {
                let mut client = pool.connect(&dsn, Duration::from_secs(5))?;
                client.batch_execute("SET statement_timeout=0; SET lock_timeout=0")?;
                let row = client.query(
                    "SELECT current_setting('statement_timeout'),current_setting('lock_timeout')",
                    &[],
                )?;
                assert_eq!(row[0].get::<_, String>(0), "80ms");
                assert_eq!(row[0].get::<_, String>(1), "40ms");
                client.batch_execute("SET statement_timeout=0")?;
                let result = if batch {
                    client.batch_execute("SELECT pg_sleep(1)")
                } else {
                    client.query("SELECT pg_sleep(1)", &[]).map(|_| ())
                };
                assert_eq!(result.err().ok_or("statement must time out")?.status, 504);
            }
            let pool = Arc::new(postgres::Pool::default());
            let mut client = pool.connect(&dsn, Duration::from_secs(2))?;
            let before: i64 = client.query(
                "SELECT setting::bigint FROM pg_settings WHERE name='statement_timeout'",
                &[],
            )?[0]
                .get(0);
            std::thread::sleep(Duration::from_millis(30));
            let after: i64 = client.query(
                "SELECT setting::bigint FROM pg_settings WHERE name='statement_timeout'",
                &[],
            )?[0]
                .get(0);
            assert!(
                after < before && before <= 2000,
                "operation budget must shrink: {before} -> {after}"
            );
            Ok(())
        }

        #[test]
        #[ignore = "requires isolated geoledger_test PostGIS database"]
        fn postgis_hot_spatial_queries_merge_stage_reuses_relation_and_clears_transaction_rows()
        -> TestResult {
            let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
            let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
            let database: String = admin.query_one("SELECT current_database()", &[])?.get(0);
            assert_eq!(database, "geoledger_test");
            let storage = Storage::Postgis(dsn);
            let pool = Arc::new(postgres::Pool::new(crate::StorageOptions {
                pool_size: 1,
                ..Default::default()
            }));
            let timeout = Duration::from_secs(30);
            let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
            t.begin_merge()?;
            let relation: String = t
                .query_one("SELECT 'pg_temp.gl_merge_stage'::regclass::oid::text", &[])?
                .get(0usize)?;
            t.stage_merge("d", "old", &None, &Some("{}".into()))?;
            t.flush_stage()?;
            t.commit()?;
            let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
            t.begin_merge()?;
            assert_eq!(
                t.query_one("SELECT 'pg_temp.gl_merge_stage'::regclass::oid::text", &[])?
                    .get::<_, String>(0usize)?,
                relation
            );
            assert_eq!(
                t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
                    .get::<_, i64>(0usize)?,
                0
            );
            t.stage_merge("d", "rollback", &None, &Some("{}".into()))?;
            t.flush_stage()?;
            drop(t);
            let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
            t.begin_merge()?;
            assert_eq!(
                t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
                    .get::<_, i64>(0usize)?,
                0
            );
            t.stage_merge("d", "pending", &None, &Some("{}".into()))?;
            t.begin_merge()?;
            t.flush_stage()?;
            assert_eq!(
                t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
                    .get::<_, i64>(0usize)?,
                0
            );
            t.commit()?;
            Ok(())
        }

        #[test]
        #[ignore = "requires isolated geoledger_test PostGIS database"]
        fn postgis_hot_spatial_queries_publication_looks_up_changed_current_rows() -> TestResult {
            let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
            let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
            let database: String = admin.query_one("SELECT current_database()", &[])?.get(0);
            assert_eq!(database, "geoledger_test");
            let storage = Storage::Postgis(dsn);
            let pool = Arc::new(postgres::Pool::default());
            let timeout = Duration::from_secs(30);
            Client::open(&storage, &pool, timeout)?.migrate()?;
            let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
            let background = uuid::Uuid::new_v4().to_string();
            let project = uuid::Uuid::new_v4().to_string();
            // Model the nightly capacity -> load sequence: statistics describe a large
            // existing project, then another project's history grows before autoanalyze.
            for (project, rows) in [(&background, 100_000_i64), (&project, 10_000)] {
                t.execute(
                    "INSERT INTO gl_projects(id,name) VALUES($1,'plan')",
                    &[&project],
                )?;
                t.execute(
            "INSERT INTO gl_project_members(project,subject,role) VALUES($1,'alice','owner')",
            &[&project],
        )?;
                t.execute(
            "INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES($1,'d','points','point')",
            &[&project],
        )?;
                t.execute(
            "INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES($1,'w','alice',0)",
            &[&project],
        )?;
                t.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES($1,1,'w','alice','seed'),($1,2,'w','alice','edit')", &[&project])?;
                t.execute("INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties) SELECT $1,'d',lpad(n::text,8,'0'),1,'{}' FROM generate_series(1,$2::bigint) n", &[&project, &rows])?;
                if project == &background {
                    t.batch_execute("ANALYZE gl_history")?;
                }
                seed_current_from_history(&mut t, project, "d")?;
            }
            t.batch_execute("SET LOCAL plan_cache_mode=force_custom_plan")?;
            t.begin_merge()?;
            t.stage_merge("d", "00000001", &Some("{}".into()), &None)?;
            t.stage_merge("d", "00000002", &Some("{}".into()), &Some("{}".into()))?;
            for n in 0..8 {
                t.stage_merge("d", &format!("new-{n}"), &None, &Some("{}".into()))?;
            }
            t.close_history(&project, 2)?;
            let table = t.dataset_table(&project, "d")?.ok_or("managed table")?;
            for mode in ["force_custom_plan", "force_generic_plan"] {
                t.batch_execute(&format!("SET LOCAL plan_cache_mode={mode}"))?;
                let plan = t
                    .query(
                        &format!("EXPLAIN (ANALYZE, COSTS OFF) SELECT valid_from FROM {table} WHERE feature_id=$1"),
                        &[&"00000001"],
                    )?
                    .iter()
                    .map(|r| r.get::<_, String>(0usize))
                    .collect::<Result<Vec<_>>>()?
                    .join("\n");
                assert!(
                    plan.contains(&format!("Index Scan using {table}_pkey")),
                    "publication lost current-key lookup ({mode}): {plan}"
                );
                assert!(
                    !plan.contains("Seq Scan"),
                    "small publication scanned the full current table: {plan}"
                );
                eprintln!("{mode}: {plan}");
            }
            assert_eq!(
                t.query_one(
                    "SELECT count(*) FROM gl_history WHERE project=$1 AND valid_to=2",
                    &[&project]
                )?
                .get::<_, i64>(0usize)?,
                2
            );
            assert_eq!(
                t.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
                    .get::<_, i64>(0usize)?,
                10_000
            );
            Ok(())
        }
    }
}
