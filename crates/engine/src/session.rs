//! Portable transaction boundary; SQL is shared, locking and spatial indexes are explicit dialect choices.
pub(crate) mod pgtls;
pub(crate) mod portable;
pub(crate) mod postgres;
mod tables;
use crate::repository::{RepositoryTransaction, StorageBackend};
use crate::{Error, FORMAT_VERSION, Result, Storage};
use rusqlite::functions::FunctionFlags;
use serde_json::value::RawValue;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) trait Parameter {
    fn value(&self) -> Cell;
}
#[derive(Clone)]
pub enum Cell {
    Null,
    Text(String),
    Integer(i64),
    Real(f64),
    Bool(bool),
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
pub trait Decode: Sized {
    fn decode(cell: &Cell) -> Option<Self>;
}
impl Decode for String {
    fn decode(c: &Cell) -> Option<Self> {
        if let Cell::Text(v) = c {
            Some(v.clone())
        } else {
            None
        }
    }
}
impl Decode for i64 {
    fn decode(c: &Cell) -> Option<Self> {
        if let Cell::Integer(v) = c {
            Some(*v)
        } else {
            None
        }
    }
}
impl Decode for i32 {
    fn decode(c: &Cell) -> Option<Self> {
        i64::decode(c).and_then(|v| i32::try_from(v).ok())
    }
}
impl Decode for bool {
    fn decode(c: &Cell) -> Option<Self> {
        match c {
            Cell::Bool(v) => Some(*v),
            Cell::Integer(v) => Some(*v != 0),
            _ => None,
        }
    }
}
impl<T: Decode> Decode for Option<T> {
    fn decode(c: &Cell) -> Option<Self> {
        if matches!(c, Cell::Null) {
            Some(None)
        } else {
            T::decode(c).map(Some)
        }
    }
}
#[derive(Clone)]
pub struct Row(Vec<Cell>);
impl Row {
    pub fn new(values: Vec<Cell>) -> Self {
        Self(values)
    }

    pub fn get<I: Into<usize>, T: Decode>(&self, index: I) -> Result<T> {
        self.0
            .get(index.into())
            .and_then(T::decode)
            .ok_or_else(|| Error::new(500, "storage row violates repository contract"))
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
}
impl Client {
    pub fn open(storage: &Storage, pool: &Arc<postgres::Pool>, timeout: Duration) -> Result<Self> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Error::new(400, "invalid deadline"))?;
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
    stage: Vec<(String, String, Option<String>, Option<String>)>,
    stage_bytes: usize,
    custom_spatial_plan: bool,
}
pub(crate) struct SqlTransaction(Client, TransactionState);
impl SqlTransaction {
    fn flush_stage(&mut self) -> Result<()> {
        if self.1.stage.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.1.stage);
        self.1.stage_bytes = 0;
        let tuples: Vec<_> = (0..rows.len())
            .map(|i| {
                format!(
                    "(${},${},${},${})",
                    i * 4 + 1,
                    i * 4 + 2,
                    i * 4 + 3,
                    i * 4 + 4
                )
            })
            .collect();
        let mut params: Vec<&dyn Parameter> = Vec::with_capacity(rows.len() * 4);
        for (dataset, key, before, after) in &rows {
            params.extend([dataset as &dyn Parameter, key, before, after]);
        }
        self.execute(
            &format!("INSERT INTO gl_merge_stage VALUES {}", tuples.join(",")),
            &params,
        )
    }

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
    fn feature_rows(
        &mut self,
        r: &crate::repository::FeatureQuery,
        include_payload: bool,
    ) -> Result<Vec<Row>> {
        let bbox = r.bbox.unwrap_or([-180., -90., 180., 90.]);
        if r.bbox.is_some()
            && matches!(self.0.backend, Backend::Postgis(_))
            && !self.1.custom_spatial_plan
        {
            // Bbox selectivity varies by orders of magnitude. Keep parameter-aware
            // plans for this transaction without disabling prepared statements globally.
            self.batch_execute("SET LOCAL plan_cache_mode=force_custom_plan")?;
            self.1.custom_spatial_plan = true;
        }
        let latest = self.1.heads.get(&r.project) == Some(&r.revision);
        let visible = if latest {
            "h.valid_to IS NULL"
        } else {
            "(h.valid_to IS NULL OR h.valid_to>$3)"
        };
        // Bound both UNION branches before geometry/JSON expansion and final
        // sorting. A top-level LIMIT alone let PostgreSQL scan and serialize
        // the entire remaining dataset for each 32-row page.
        let key_filter = if r.feature_id.is_some() {
            "feature_id=$6"
        } else {
            "CAST($6 AS text) IS NULL"
        };
        let sparse_history = self.sparse_spatial_candidates("history", r)?;
        let sparse_draft = self.sparse_spatial_candidates("workspace_changes", r)?;
        let spatial_filter = self.spatial_filter("h", "history", r.bbox.is_some(), sparse_history);
        let draft_spatial_filter =
            self.spatial_filter("", "workspace_changes", r.bbox.is_some(), sparse_draft);
        let (spatial_lookup, draft_lookup) = if matches!(self.0.backend, Backend::Sqlite(_)) {
            (
                if latest {
                    " INDEXED BY history_live"
                } else {
                    " INDEXED BY sqlite_autoindex_gl_history_1"
                },
                " INDEXED BY sqlite_autoindex_gl_workspace_changes_1",
            )
        } else {
            ("", "")
        };
        let base_payload = if include_payload {
            ",h.properties,h.geometry_json"
        } else {
            ""
        };
        let draft_payload = if include_payload {
            ",properties,geometry_json"
        } else {
            ""
        };
        let sql = format!("WITH base AS MATERIALIZED (
            SELECT h.feature_id{base_payload} FROM gl_history h{spatial_lookup}
            WHERE h.project=$1 AND h.dataset=$2
              AND h.valid_from<=$3 AND {visible}
              AND h.properties IS NOT NULL AND h.feature_id>$5 AND {key_filter}
              AND ({spatial_filter})
              AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c
                WHERE c.project=h.project AND c.dataset=h.dataset AND c.feature_id=h.feature_id AND c.workspace=$4)
            ORDER BY h.feature_id LIMIT $12
          ), draft AS MATERIALIZED (
            SELECT feature_id{draft_payload} FROM gl_workspace_changes{draft_lookup}
            WHERE project=$1 AND dataset=$2 AND workspace=$4
              AND properties IS NOT NULL AND feature_id>$5 AND {key_filter}
              AND ({draft_spatial_filter})
            ORDER BY feature_id LIMIT $12
          ), page AS MATERIALIZED (
            SELECT * FROM base UNION ALL SELECT * FROM draft ORDER BY feature_id LIMIT $12
          ) SELECT * FROM page ORDER BY feature_id");
        self.query(
            &sql,
            &[
                &r.project,
                &r.dataset,
                &r.revision,
                &r.workspace,
                &r.after,
                &r.feature_id,
                &r.bbox.is_some(),
                &bbox[0],
                &bbox[1],
                &bbox[2],
                &bbox[3],
                &r.limit,
            ],
        )
    }
    fn sparse_spatial_candidates(
        &mut self,
        table: &str,
        r: &crate::repository::FeatureQuery,
    ) -> Result<bool> {
        let Some(bbox) = r.bbox else { return Ok(false) };
        if r.feature_id.is_some() || !matches!(self.0.backend, Backend::Sqlite(_)) {
            return Ok(false);
        }
        // Small Rtree results avoid even the bounded feature-key probe below.
        let cap = r.limit.saturating_mul(4);
        let count: i64 = self.query_one(&format!("SELECT count(*) FROM (
            SELECT pkid FROM idx_gl_{table}_geom WHERE xmin<=$3 AND xmax>=$1 AND ymin<=$4 AND ymax>=$2
            UNION ALL SELECT pkid FROM idx_gl_{table}_geom_z WHERE xmin<=$3 AND xmax>=$1 AND ymin<=$4 AND ymax>=$2 LIMIT $5
        )"), &[&bbox[0], &bbox[1], &bbox[2], &bbox[3], &cap])?.get(0usize)?;
        if count < cap {
            return Ok(true);
        }
        // Global Rtree density says nothing about this dataset or cursor. Use
        // key order when a bounded prefix fills the page or exhausts the remaining
        // keys, so terminal lookahead does not enumerate a dense global Rtree.
        let (scope, visible, visibility_columns) = if table == "history" {
            (
                "project=$1 AND dataset=$2 AND feature_id>$5",
                "valid_from<=$3 AND (valid_to IS NULL OR valid_to>$3) AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c INDEXED BY sqlite_autoindex_gl_workspace_changes_1 WHERE c.project=$1 AND c.workspace=$4 AND c.dataset=$2 AND c.feature_id=prefix.feature_id)",
                ",valid_from,valid_to",
            )
        } else {
            (
                "project=$1 AND workspace=$4 AND dataset=$2 AND feature_id>$5",
                "CAST($3 AS bigint) IS NOT NULL",
                "",
            )
        };
        let row = self.query_one(
            &format!("WITH prefix AS MATERIALIZED (SELECT feature_id,properties IS NOT NULL AS present,geom,geom_z{visibility_columns} FROM gl_{table} INDEXED BY sqlite_autoindex_gl_{table}_1 WHERE {scope} ORDER BY feature_id LIMIT $10) SELECT (SELECT count(*) FROM (SELECT 1 FROM prefix WHERE present AND ({visible}) AND (ST_Intersects(geom,BuildMbr($6,$7,$8,$9,0))=1 OR ST_Intersects(geom_z,BuildMbr($6,$7,$8,$9,0))=1) LIMIT $11)), (SELECT count(*) FROM prefix)"),
            &[&r.project, &r.dataset, &r.revision, &r.workspace, &r.after, &bbox[0], &bbox[1], &bbox[2], &bbox[3], &cap, &r.limit],
        )?;
        let matches: i64 = row.get(0usize)?;
        let remaining: i64 = row.get(1usize)?;
        Ok(matches < r.limit && remaining >= cap)
    }
    pub(crate) fn spatial_filter(
        &self,
        alias: &str,
        table: &str,
        enabled: bool,
        candidates: bool,
    ) -> String {
        // Cast unused parameters too: PostgreSQL prepares a single typed parameter vector.
        if !enabled {
            return "NOT $7 AND CAST($8 AS double precision) IS NOT NULL AND CAST($9 AS double precision) IS NOT NULL AND CAST($10 AS double precision) IS NOT NULL AND CAST($11 AS double precision) IS NOT NULL".into();
        }
        let a = if alias.is_empty() {
            String::new()
        } else {
            format!("{alias}.")
        };
        match self.0.backend {
            Backend::Postgis(_) => {
                format!("$7 AND ST_Intersects({a}geom,ST_MakeEnvelope($8,$9,$10,$11,0))")
            }
            Backend::Sqlite(_) if !candidates => format!(
                "$7 AND (ST_Intersects({a}geom,BuildMbr($8,$9,$10,$11,0))=1 OR ST_Intersects({a}geom_z,BuildMbr($8,$9,$10,$11,0))=1)"
            ),
            // +0 retains scalar rowid point lookup even after a stale one-row ANALYZE estimate.
            Backend::Sqlite(_) => format!(
                "$7 AND {a}feature_id IN (SELECT (SELECT feature_id FROM gl_{table} WHERE rowid=spatial_candidates.pkid+0) FROM (SELECT pkid FROM idx_gl_{table}_geom WHERE xmin<=$10 AND xmax>=$8 AND ymin<=$11 AND ymax>=$9 UNION ALL SELECT pkid FROM idx_gl_{table}_geom_z WHERE xmin<=$10 AND xmax>=$8 AND ymin<=$11 AND ymax>=$9) spatial_candidates) AND (ST_Intersects({a}geom,BuildMbr($8,$9,$10,$11,0))=1 OR ST_Intersects({a}geom_z,BuildMbr($8,$9,$10,$11,0))=1)"
            ),
        }
    }
    fn workspace_key_table(&self) -> &'static str {
        match self.0.backend {
            // ANALYZE on a new database can retain a one-row estimate after drafts grow.
            Backend::Sqlite(_) => {
                "gl_workspace_changes INDEXED BY sqlite_autoindex_gl_workspace_changes_1"
            }
            Backend::Postgis(_) => "gl_workspace_changes",
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
impl crate::repository::RepositoryTransaction for SqlTransaction {
    fn lock_credentials(&mut self) -> Result<()> {
        if matches!(self.0.backend, Backend::Postgis(_)) {
            self.query_one("SELECT pg_advisory_xact_lock(7885)::text", &[])?;
        }
        Ok(())
    }
    fn auth_user(&mut self, id: &str) -> Result<Option<crate::authentication::UserRecord>> {
        self.query_opt(
            &format!("{} WHERE u.id=$1", authentication::USER_SELECT),
            &[&id],
        )?
        .map(authentication::user_row)
        .transpose()
    }
    fn auth_username(
        &mut self,
        username: &str,
    ) -> Result<Option<crate::authentication::UserRecord>> {
        self.query_opt(
            &format!("{} WHERE u.username=$1", authentication::USER_SELECT),
            &[&username],
        )?
        .map(authentication::user_row)
        .transpose()
    }
    fn auth_users(&mut self, after: &str, limit: i64, status: &str) -> Result<Vec<crate::User>> {
        let select = "SELECT id,username,display_name,status,is_admin,must_change_password,created_at FROM gl_users";
        let rows = if status.is_empty() {
            self.query(
                &format!("{select} WHERE id>$1 ORDER BY id LIMIT $2"),
                &[&after, &limit],
            )?
        } else {
            self.query(
                &format!("{select} WHERE status=$3 AND id>$1 ORDER BY id LIMIT $2"),
                &[&after, &limit, &status],
            )?
        };
        rows.into_iter()
            .map(|r| authentication::public_user_row(&r))
            .collect()
    }
    fn auth_insert_user(&mut self, r: &crate::authentication::UserRecord) -> Result<()> {
        let u = &r.user;
        self.execute("INSERT INTO gl_users(id,username,display_name,status,is_admin,must_change_password,created_at,credential_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&u.id,&u.username,&u.display_name,&u.status,&u.is_admin,&u.must_change_password,&u.created_at,&r.credential_version])?;
        self.execute(
            "INSERT INTO gl_passwords(user_id,password_hash) VALUES($1,$2)",
            &[&u.id, &r.password_hash],
        )
    }
    fn auth_update_user(&mut self, r: &crate::authentication::UserRecord) -> Result<()> {
        let u = &r.user;
        self.execute("UPDATE gl_users SET username=$2,display_name=$3,status=$4,is_admin=$5,must_change_password=$6,credential_version=$7 WHERE id=$1", &[&u.id,&u.username,&u.display_name,&u.status,&u.is_admin,&u.must_change_password,&r.credential_version])?;
        self.execute(
            "UPDATE gl_passwords SET password_hash=$2 WHERE user_id=$1",
            &[&u.id, &r.password_hash],
        )
    }
    fn auth_admin_count(&mut self) -> Result<i64> {
        self.query_one(
            "SELECT count(*) FROM gl_users WHERE status='active' AND is_admin",
            &[],
        )?
        .get(0usize)
    }
    fn auth_membership_target(&mut self, id: &str) -> Result<bool> {
        let suffix = if matches!(self.0.backend, Backend::Postgis(_)) {
            " FOR SHARE"
        } else {
            ""
        };
        Ok(self
            .query_opt(
                &format!("SELECT id FROM gl_users WHERE id=$1 AND status='active'{suffix}"),
                &[&id],
            )?
            .is_some())
    }
    fn auth_user_count(&mut self) -> Result<i64> {
        self.query_one("SELECT count(*) FROM gl_users", &[])?
            .get(0usize)
    }
    fn auth_key(&mut self, id: &str) -> Result<Option<crate::ApiKey>> {
        self.query_opt(
            &format!("{} WHERE id=$1", authentication::KEY_SELECT),
            &[&id],
        )?
        .map(authentication::key_row)
        .transpose()
    }
    fn auth_key_digest(&mut self, digest: &str, now: &str) -> Result<Option<crate::ApiKey>> {
        self.query_opt(&format!("{} WHERE digest=$1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>$2)",authentication::KEY_SELECT), &[&digest,&now])?.map(authentication::key_row).transpose()
    }
    fn auth_keys(&mut self, user: &str, after: &str, limit: i64) -> Result<Vec<crate::ApiKey>> {
        self.query(
            &format!(
                "{} WHERE user_id=$1 AND id>$2 ORDER BY id LIMIT $3",
                authentication::KEY_SELECT
            ),
            &[&user, &after, &limit],
        )?
        .into_iter()
        .map(authentication::key_row)
        .collect()
    }
    fn auth_insert_key(&mut self, k: &crate::ApiKey, digest: &str) -> Result<()> {
        self.execute("INSERT INTO gl_api_keys(id,user_id,digest,name,key_prefix,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&k.id,&k.user_id,&digest,&k.name,&k.key_prefix,&k.created_at,&k.expires_at])
    }
    fn auth_update_key(&mut self, k: &crate::ApiKey) -> Result<()> {
        self.execute(
            "UPDATE gl_api_keys SET name=$2,expires_at=$3,revoked_at=$4 WHERE id=$1",
            &[&k.id, &k.name, &k.expires_at, &k.revoked_at],
        )
    }
    fn auth_delete_key(&mut self, id: &str) -> Result<()> {
        self.execute("DELETE FROM gl_api_keys WHERE id=$1", &[&id])
    }
    fn auth_touch_key(&mut self, id: &str, now: &str, before: &str) -> Result<()> {
        self.execute("UPDATE gl_api_keys SET last_used_at=$2 WHERE id=$1 AND (last_used_at IS NULL OR last_used_at<$3)", &[&id,&now,&before])
    }
    fn auth_session(
        &mut self,
        digest: &str,
    ) -> Result<Option<crate::authentication::SessionRecord>> {
        self.query_opt("SELECT id,user_id,source_key_id,credential_version,created_at,last_seen_at,expires_at FROM gl_sessions WHERE digest=$1", &[&digest])?.map(|r| Ok(crate::authentication::SessionRecord {id:r.get(0usize)?,user_id:r.get(1usize)?,source_key_id:r.get(2usize)?,credential_version:r.get(3usize)?,created_at:r.get(4usize)?,last_seen_at:r.get(5usize)?,expires_at:r.get(6usize)?})).transpose()
    }
    fn auth_insert_session(
        &mut self,
        s: &crate::authentication::SessionRecord,
        digest: &str,
    ) -> Result<()> {
        // Bound cleanup work per login; the expiry index avoids scanning live sessions.
        self.execute(
            "DELETE FROM gl_sessions WHERE id IN (SELECT id FROM gl_sessions WHERE expires_at<=$1 ORDER BY expires_at LIMIT 100)",
            &[&s.created_at],
        )?;
        self.execute("INSERT INTO gl_sessions(id,user_id,digest,source_key_id,credential_version,created_at,last_seen_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&s.id,&s.user_id,&digest,&s.source_key_id,&s.credential_version,&s.created_at,&s.last_seen_at,&s.expires_at])
    }
    fn auth_touch_session(&mut self, id: &str, now: &str, before: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_sessions SET last_seen_at=$2 WHERE id=$1 AND last_seen_at<$3",
            &[&id, &now, &before],
        )
    }
    fn auth_delete_session(&mut self, id: &str) -> Result<()> {
        self.execute("DELETE FROM gl_sessions WHERE id=$1", &[&id])
    }
    fn auth_invalidate_user(&mut self, id: &str, now: &str, keys: bool) -> Result<()> {
        if keys {
            self.execute(
                "UPDATE gl_api_keys SET revoked_at=$2 WHERE user_id=$1 AND revoked_at IS NULL",
                &[&id, &now],
            )?;
        }
        self.execute("DELETE FROM gl_sessions WHERE user_id=$1", &[&id])
    }
    fn auth_audit(&mut self, actor: &str, action: &str, resource: &str, now: &str) -> Result<()> {
        self.execute(
            "INSERT INTO gl_auth_audit(id,actor,action,resource,created_at) VALUES($1,$2,$3,$4,$5)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &actor,
                &action,
                &resource,
                &now,
            ],
        )
    }

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

    fn member_role(&mut self, project: &str, subject: &str) -> Result<Option<Row>> {
        self.query_opt(
            &self.lock_sql(
                "SELECT m.role,p.state FROM gl_project_members m JOIN gl_projects p ON p.id=m.project WHERE m.project=$1 AND m.subject=$2 AND NOT m.removed AND p.state<>'deleted'",
                "FOR SHARE OF m",
            ),
            &[&project, &subject],
        )
    }
    fn project_head(&mut self, project: &str, lock: bool) -> Result<Option<Row>> {
        let sql = "SELECT head FROM gl_projects WHERE id=$1";
        let row = self.query_opt(
            &self.lock_sql(sql, if lock { "FOR NO KEY UPDATE" } else { "" }),
            &[&project],
        )?;
        if let Some(row) = &row {
            self.1.heads.insert(project.to_owned(), row.get(0usize)?);
        }
        Ok(row)
    }
    fn project_head_for_edit(&mut self, project: &str) -> Result<Option<Row>> {
        let row = self.query_opt(
            &self.lock_sql("SELECT head FROM gl_projects WHERE id=$1", "FOR SHARE"),
            &[&project],
        )?;
        if let Some(row) = &row {
            self.1.heads.insert(project.to_owned(), row.get(0usize)?);
        }
        Ok(row)
    }
    fn workspace_state(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        write: bool,
    ) -> Result<Option<Row>> {
        self.query_opt(&self.lock_sql("SELECT base_revision,version,status FROM gl_workspaces WHERE project=$1 AND id=$2 AND owner=$3",if write {"FOR UPDATE"}else{"FOR SHARE"}), &[&project,&workspace,&subject])
    }
    fn feature_page(&mut self, r: &crate::repository::FeatureQuery) -> Result<Vec<Row>> {
        self.feature_rows(r, true)
    }
    fn feature_exists(&mut self, r: &crate::repository::FeatureQuery) -> Result<bool> {
        let query = crate::repository::FeatureQuery {
            limit: 1,
            ..r.clone()
        };
        Ok(!self.feature_rows(&query, false)?.is_empty())
    }
    fn feature_at(
        &mut self,
        project: &str,
        dataset: &str,
        key: &str,
        revision: i64,
    ) -> Result<Option<Row>> {
        self.query_opt("SELECT properties,geometry_json FROM gl_history WHERE project=$1 AND dataset=$2 AND feature_id=$3 AND valid_from<=$4 AND (valid_to IS NULL OR valid_to>$4) ORDER BY valid_from DESC LIMIT 1", &[&project, &dataset, &key, &revision])
    }
    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>> {
        let key = (project.to_owned(), dataset.to_owned());
        if let Some(row) = self.1.datasets.get(&key) {
            return Ok(Some(row.clone()));
        }
        let row = self.query_opt(
            "SELECT geometry_type,name,coordinate_dimension,postgis_source,crs FROM gl_datasets WHERE project=$1 AND id=$2",
            &[&project, &dataset],
        )?;
        if let Some(row) = &row {
            if self.1.datasets.len() >= 128 {
                self.1.datasets.clear();
            }
            self.1.datasets.insert(key, row.clone());
        }
        Ok(row)
    }
    fn append_audit(
        &mut self,
        project: &str,
        subject: &str,
        action: &str,
        detail: &str,
    ) -> Result<()> {
        if matches!(self.0.backend, Backend::Postgis(_)) {
            // Hold through commit so project cursors cannot skip a lower, uncommitted ID.
            self.query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,7882))::text",
                &[&project],
            )?;
        }
        self.execute(
            "INSERT INTO gl_audit_events(project,subject,action,detail) VALUES($1,$2,$3,$4)",
            &[&project, &subject, &action, &detail],
        )
    }
    fn advance_workspace(&mut self, project: &str, workspace: &str, status: &str) -> Result<Row> {
        self.query_one("UPDATE gl_workspaces SET version=version+1,status=$3 WHERE project=$1 AND id=$2 RETURNING version", &[&project, &workspace, &status])
    }
    fn insert_project(&mut self, project: &str, name: &str) -> Result<()> {
        self.execute(
            "INSERT INTO gl_projects(id,name) VALUES($1,$2)",
            &[&project, &name],
        )
    }
    fn insert_owner(&mut self, project: &str, subject: &str) -> Result<()> {
        self.execute(
            "INSERT INTO gl_project_members(project,subject,role) VALUES($1,$2,'owner')",
            &[&project, &subject],
        )
    }
    fn list_projects(&mut self, subject: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT p.id,p.name,p.head,p.state,m.role FROM gl_projects p JOIN gl_project_members m ON m.project=p.id WHERE m.subject=$1 AND NOT m.removed AND p.state<>'deleted' AND p.id>$2 ORDER BY p.id LIMIT $3", &[&subject, &after, &limit])
    }
    fn project_info(&mut self, project: &str) -> Result<Row> {
        self.query_one(
            "SELECT name,head,state FROM gl_projects WHERE id=$1",
            &[&project],
        )
    }
    fn owner_summary(&mut self, project: &str, subject: &str) -> Result<Row> {
        self.query_one("SELECT count(*) FILTER (WHERE role='owner'), coalesce(max(CASE WHEN subject=$2 AND role='owner' THEN 1 ELSE 0 END),0) FROM gl_project_members WHERE project=$1 AND NOT removed", &[&project, &subject])
    }
    fn set_member(&mut self, project: &str, subject: &str, role: &str) -> Result<()> {
        self.execute("INSERT INTO gl_project_members(project,subject,role) VALUES($1,$2,$3) ON CONFLICT(project,subject) DO UPDATE SET role=excluded.role,removed=false", &[&project, &subject, &role])
    }
    fn live_project(&mut self, project: &str, lock: bool) -> Result<Option<Row>> {
        let sql = "SELECT name,head,state FROM gl_projects WHERE id=$1 AND state<>'deleted'";
        if lock {
            self.query_opt(&self.lock_sql(sql, "FOR UPDATE"), &[&project])
        } else {
            self.query_opt(sql, &[&project])
        }
    }
    fn list_members(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT subject,role FROM gl_project_members WHERE project=$1 AND NOT removed AND subject>$2 ORDER BY subject LIMIT $3", &[&project, &after, &limit])
    }
    fn remove_member(&mut self, project: &str, subject: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_project_members SET removed=true WHERE project=$1 AND subject=$2",
            &[&project, &subject],
        )
    }
    fn set_project_state(&mut self, project: &str, state: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_projects SET state=$2 WHERE id=$1",
            &[&project, &state],
        )
    }
    fn owned_projects(&mut self, subject: &str) -> Result<Row> {
        if matches!(self.0.backend, Backend::Postgis(_)) {
            // Serialize quota checks through commit, including when the subject has no projects.
            self.query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,7883))::text",
                &[&subject],
            )?;
        }
        self.query_one("SELECT count(*) FROM gl_project_members m JOIN gl_projects p ON p.id=m.project WHERE m.subject=$1 AND m.role='owner' AND NOT m.removed AND p.state<>'deleted'", &[&subject])
    }
    fn ensure_identity(&mut self, project: &str, subject: &str) -> Result<()> {
        // Audit rows reference a member row; administrators acting without
        // membership get an inactive placeholder that grants nothing.
        self.execute("INSERT INTO gl_project_members(project,subject,role,removed) VALUES($1,$2,'viewer',true) ON CONFLICT(project,subject) DO NOTHING", &[&project, &subject])
    }
    fn insert_dataset(
        &mut self,
        project: &str,
        dataset: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: i32,
        crs: &str,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO gl_datasets(project,id,name,geometry_type,coordinate_dimension,crs) VALUES($1,$2,$3,$4,$5,$6)",
            &[
                &project,
                &dataset,
                &name,
                &geometry_type,
                &coordinate_dimension,
                &crs,
            ],
        )
    }
    fn inspect_postgis_table(&mut self, source: &crate::PostgisTable) -> Result<Row> {
        let metadata = self.inspect_source(source)?;
        Ok(Row::new(vec![
            Cell::Text(metadata.family),
            Cell::Integer(metadata.dimension.into()),
            Cell::Text(metadata.crs),
        ]))
    }
    fn postgis_table_page(
        &mut self,
        source: &crate::PostgisTable,
        after: &str,
    ) -> Result<Vec<Row>> {
        self.source_page(source, after)
    }
    fn bind_postgis_table(
        &mut self,
        project: &str,
        dataset: &str,
        source: &crate::PostgisTable,
    ) -> Result<()> {
        let source_metadata = self.inspect_source(source)?;
        let metadata = serde_json::to_string(&source_metadata).map_err(Error::stored_json)?;
        self.execute(
            "UPDATE gl_datasets SET postgis_source=$3 WHERE project=$1 AND id=$2",
            &[&project, &dataset, &metadata],
        )?;
        self.1.datasets.clear();
        self.guard_source(source, source_metadata.srid)
    }
    fn rename_project(&mut self, project: &str, name: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_projects SET name=$2 WHERE id=$1",
            &[&project, &name],
        )
    }
    fn rename_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()> {
        self.1.datasets.clear();
        self.execute(
            "UPDATE gl_datasets SET name=$3 WHERE project=$1 AND id=$2",
            &[&project, &dataset, &name],
        )
    }
    fn purge_data(&mut self, project: &str, dataset: Option<&str>) -> Result<()> {
        self.1.datasets.clear();
        self.release_sources(project, dataset)?;
        self.execute("INSERT INTO gl_purge VALUES($1)", &[&project])?;
        if let Some(dataset) = dataset {
            // Capture only workspaces touched by this dataset, including published drafts.
            let affected = self.query("SELECT workspace FROM gl_workspace_changes WHERE project=$1 AND dataset=$2 UNION SELECT c.workspace FROM gl_commits c JOIN gl_commit_changes d ON c.project=d.project AND c.revision=d.revision WHERE d.project=$1 AND d.dataset=$2", &[&project, &dataset])?;
            let commits = self.query(
                "SELECT DISTINCT revision FROM gl_commit_changes WHERE project=$1 AND dataset=$2",
                &[&project, &dataset],
            )?;
            for table in ["gl_workspace_changes", "gl_history", "gl_commit_changes"] {
                self.execute(
                    &format!("DELETE FROM {table} WHERE project=$1 AND dataset=$2"),
                    &[&project, &dataset],
                )?;
            }
            for row in commits {
                let revision: i64 = row.get(0usize)?;
                let remaining: i64 = self
                    .query_one(
                        "SELECT count(*) FROM gl_commit_changes WHERE project=$1 AND revision=$2",
                        &[&project, &revision],
                    )?
                    .get(0usize)?;
                if remaining == 0 {
                    let revision_text = revision.to_string();
                    self.execute("DELETE FROM gl_idempotency WHERE project=$1 AND gl_json_field(result,'revision')=$2", &[&project, &revision_text])?;
                    self.execute(
                        "DELETE FROM gl_commits WHERE project=$1 AND revision=$2",
                        &[&project, &revision],
                    )?;
                }
            }
            for row in affected {
                let workspace: String = row.get(0usize)?;
                let remaining: i64 = self.query_one("SELECT (SELECT count(*) FROM gl_workspace_changes WHERE project=$1 AND workspace=$2)+(SELECT count(*) FROM gl_commits WHERE project=$1 AND workspace=$2)", &[&project, &workspace])?.get(0usize)?;
                if remaining == 0 {
                    self.execute("DELETE FROM gl_idempotency WHERE project=$1 AND gl_json_field(payload,'workspace')=$2", &[&project, &workspace])?;
                    self.execute(
                        "DELETE FROM gl_workspaces WHERE project=$1 AND id=$2",
                        &[&project, &workspace],
                    )?;
                } else {
                    self.invalidate_resolutions(project, &workspace)?;
                    self.execute(
                        "UPDATE gl_workspaces SET version=version+1 WHERE project=$1 AND id=$2",
                        &[&project, &workspace],
                    )?;
                }
            }
            self.execute(
                "DELETE FROM gl_datasets WHERE project=$1 AND id=$2",
                &[&project, &dataset],
            )?;
        } else {
            for table in [
                "gl_workspace_changes",
                "gl_history",
                "gl_commit_changes",
                "gl_idempotency",
                "gl_commits",
                "gl_workspaces",
                "gl_datasets",
            ] {
                self.execute(
                    &format!("DELETE FROM {table} WHERE project=$1"),
                    &[&project],
                )?;
            }
        }
        // The guard is transaction-local in lifetime; errors roll it and all deletes back.
        self.execute("DELETE FROM gl_purge WHERE project=$1", &[&project])
    }
    fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query(
            "SELECT id,name,geometry_type,coordinate_dimension,postgis_source,crs FROM gl_datasets WHERE project=$1 AND id>$2 ORDER BY id LIMIT $3",
            &[&project, &after, &limit],
        )
    }
    fn insert_workspace(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        base: i64,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES($1,$2,$3,$4)",
            &[&project, &workspace, &subject, &base],
        )
    }
    fn list_workspaces(
        &mut self,
        project: &str,
        subject: &str,
        after: &str,
        limit: i64,
    ) -> Result<Vec<Row>> {
        self.query("SELECT id,base_revision,version,status FROM gl_workspaces WHERE project=$1 AND owner=$2 AND id>$3 ORDER BY id LIMIT $4", &[&project, &subject, &after, &limit])
    }
    fn put_deltas(
        &mut self,
        project: &str,
        workspace: &str,
        deltas: &[crate::repository::DraftDelta],
    ) -> Result<()> {
        if deltas.is_empty() {
            return Ok(());
        }
        let updates = self
            .geometry_columns()
            .split(',')
            .map(|c| format!("{c}=excluded.{c}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut params: Vec<&dyn Parameter> = vec![&project, &workspace];
        let mut values = Vec::with_capacity(deltas.len());
        for (i, delta) in deltas.iter().enumerate() {
            let n = i * 4 + 3;
            values.push(format!(
                "($1,$2,${n},${},${},${},{})",
                n + 1,
                n + 2,
                n + 3,
                self.geometry_values(&format!("${}", n + 3))
            ));
            params.extend([
                &delta.dataset as &dyn Parameter,
                &delta.key,
                &delta.properties,
                &delta.geometry,
            ]);
        }
        self.execute(&format!("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geometry_json,{columns}) VALUES {values} ON CONFLICT(project,workspace,dataset,feature_id) DO UPDATE SET properties=excluded.properties,geometry_json=excluded.geometry_json,{updates}", columns=self.geometry_columns(), values=values.join(",")), &params)
    }
    fn invalidate_resolutions(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute("UPDATE gl_workspace_changes SET resolution_stale=true WHERE project=$1 AND workspace=$2 AND resolved_head IS NOT NULL AND NOT resolution_stale", &[&project, &workspace])
    }
    fn edit_base(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        base: i64,
    ) -> Result<Row> {
        // Stale SQLite statistics must not turn a single-key lookup into a history scan.
        let history = match self.0.backend {
            Backend::Sqlite(_) => "gl_history h INDEXED BY sqlite_autoindex_gl_history_1",
            Backend::Postgis(_) => "gl_history h",
        };
        self.query_one(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4 AND resolved_head IS NOT NULL),h.properties,h.geometry_json FROM (SELECT 1) seed LEFT JOIN (SELECT properties,geometry_json FROM {history} WHERE h.project=$1 AND h.dataset=$3 AND h.feature_id=$4 AND h.valid_from<=$5 AND (h.valid_to IS NULL OR h.valid_to>$5) ORDER BY h.valid_from DESC LIMIT 1) h ON true", self.workspace_key_table()), &[&project, &workspace, &dataset, &key, &base])
    }
    fn remove_delta(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
    ) -> Result<()> {
        self.execute(
            &format!(
                "DELETE FROM {} WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4",
                self.workspace_key_table()
            ),
            &[&project, &workspace, &dataset, &key],
        )
    }
    fn count_deltas(&mut self, project: &str, workspace: &str) -> Result<Row> {
        self.query_one(
            "SELECT coalesce((SELECT changes FROM gl_workspace_sizes WHERE project=$1 AND workspace=$2),0)",
            &[&project, &workspace],
        )
    }
    fn workspace_summary(&mut self, project: &str, workspace: &str, base: i64) -> Result<Vec<Row>> {
        // Aggregate in primary-key dataset order before joining names. Grouping by
        // names too makes SQLite sort full feature payloads into a temporary B-tree.
        self.query("WITH counts AS MATERIALIZED (SELECT c.dataset,SUM(CASE WHEN b.properties IS NULL AND c.properties IS NOT NULL THEN 1 ELSE 0 END) AS added,SUM(CASE WHEN b.properties IS NOT NULL AND c.properties IS NULL THEN 1 ELSE 0 END) AS deleted,SUM(CASE WHEN b.properties IS NOT NULL AND c.properties IS NOT NULL THEN 1 ELSE 0 END) AS modified FROM gl_workspace_changes c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) WHERE c.project=$1 AND c.workspace=$2 AND (c.properties IS NOT NULL OR b.properties IS NOT NULL) GROUP BY c.dataset) SELECT c.dataset,d.name,c.added,c.deleted,c.modified FROM counts c JOIN gl_datasets d ON d.project=$1 AND d.id=c.dataset ORDER BY c.dataset", &[&project,&workspace,&base])
    }
    fn commit_summary(&mut self, project: &str, revision: i64) -> Result<Vec<Row>> {
        self.query("WITH counts AS MATERIALIZED (SELECT dataset,SUM(CASE WHEN before_value IS NULL AND after_value IS NOT NULL THEN 1 ELSE 0 END) AS added,SUM(CASE WHEN before_value IS NOT NULL AND after_value IS NULL THEN 1 ELSE 0 END) AS deleted,SUM(CASE WHEN before_value IS NOT NULL AND after_value IS NOT NULL THEN 1 ELSE 0 END) AS modified FROM gl_commit_changes WHERE project=$1 AND revision=$2 AND (before_value IS NOT NULL OR after_value IS NOT NULL) GROUP BY dataset) SELECT c.dataset,d.name,c.added,c.deleted,c.modified FROM counts c JOIN gl_datasets d ON d.project=$1 AND d.id=c.dataset ORDER BY c.dataset", &[&project,&revision])
    }
    fn diff_page(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>> {
        self.query("WITH batch AS MATERIALIZED (SELECT project,dataset,feature_id,properties,geometry_json,resolved_head,resolution_stale FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($4,$5) ORDER BY dataset,feature_id LIMIT $6) SELECT c.dataset,c.feature_id,c.properties,c.geometry_json,b.properties,b.geometry_json FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) ORDER BY c.dataset,c.feature_id", &[&project, &workspace, &base, &after_dataset, &after_key, &limit])
    }
    fn history_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT c.revision,c.subject,c.message,c.created_at,c.workspace,w.base_revision FROM gl_commits c JOIN gl_workspaces w ON w.project=c.project AND w.id=c.workspace WHERE c.project=$1 AND c.revision>$2 ORDER BY c.revision LIMIT $3", &[&project, &after, &limit])
    }
    fn commit_exists(&mut self, project: &str, revision: i64) -> Result<Option<Row>> {
        self.query_opt(
            "SELECT 1 FROM gl_commits WHERE project=$1 AND revision=$2",
            &[&project, &revision],
        )
    }
    fn commit_page(
        &mut self,
        project: &str,
        revision: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>> {
        self.query("SELECT dataset,feature_id,(gl_json_field(before_value,'properties')),gl_json_field(before_value,'geometry'),(gl_json_field(after_value,'properties')),gl_json_field(after_value,'geometry') FROM gl_commit_changes WHERE project=$1 AND revision=$2 AND (dataset,feature_id)>($3,$4) ORDER BY dataset,feature_id LIMIT $5", &[&project, &revision, &after_dataset, &after_key, &limit])
    }
    fn audit_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT id,subject,action,detail,created_at FROM gl_audit_events WHERE project=$1 AND id>$2 ORDER BY id LIMIT $3", &[&project, &after, &limit])
    }
    fn cached_conflict_count(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        version: i64,
        head: i64,
    ) -> Result<Option<i64>> {
        self.query_opt("SELECT total FROM gl_conflict_cache WHERE project=$1 AND workspace=$2 AND base_revision=$3 AND workspace_version=$4 AND head=$5 AND complete", &[&project,&workspace,&base,&version,&head])?.map(|r|r.get(0usize)).transpose()
    }
    fn reset_conflict_cache(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        version: i64,
        head: i64,
    ) -> Result<()> {
        self.execute(
            "DELETE FROM gl_conflict_cache WHERE project=$1 AND workspace=$2",
            &[&project, &workspace],
        )?;
        self.execute("INSERT INTO gl_conflict_cache(project,workspace,base_revision,workspace_version,head) VALUES($1,$2,$3,$4,$5)", &[&project,&workspace,&base,&version,&head])
    }
    fn cache_conflict_keys(
        &mut self,
        project: &str,
        workspace: &str,
        keys: &[(String, String)],
    ) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut params: Vec<&dyn Parameter> = vec![&project, &workspace];
        let mut tuples = Vec::with_capacity(keys.len());
        for (i, (dataset, key)) in keys.iter().enumerate() {
            tuples.push(format!("($1,$2,${},${})", i * 2 + 3, i * 2 + 4));
            params.extend([dataset as &dyn Parameter, key]);
        }
        self.execute(
            &format!("INSERT INTO gl_conflict_keys VALUES {}", tuples.join(",")),
            &params,
        )
    }
    fn complete_conflict_cache(
        &mut self,
        project: &str,
        workspace: &str,
        total: i64,
    ) -> Result<()> {
        self.execute(
            "UPDATE gl_conflict_cache SET total=$3,complete=true WHERE project=$1 AND workspace=$2",
            &[&project, &workspace, &total],
        )
    }
    fn cached_conflict_page(
        &mut self,
        query: &crate::repository::ConflictQuery<'_>,
    ) -> Result<Vec<Row>> {
        let crate::repository::ConflictQuery {
            project,
            workspace,
            base,
            head,
            after_dataset,
            after_key,
            limit,
        } = *query;
        self.query("WITH keys AS MATERIALIZED (SELECT dataset,feature_id FROM gl_conflict_keys WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($5,$6) ORDER BY dataset,feature_id LIMIT $7), batch AS MATERIALIZED (SELECT c.* FROM keys k JOIN gl_workspace_changes c ON c.project=$1 AND c.workspace=$2 AND c.dataset=k.dataset AND c.feature_id=k.feature_id) SELECT c.dataset,c.feature_id,c.properties,c.geometry_json,c.resolved_head,c.resolution_stale,b.properties,b.geometry_json,o.properties,o.geometry_json FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) LEFT JOIN gl_history o ON o.project=c.project AND o.dataset=c.dataset AND o.feature_id=c.feature_id AND o.valid_from<=$4 AND (o.valid_to IS NULL OR o.valid_to>$4) ORDER BY c.dataset,c.feature_id", &[&project,&workspace,&base,&head,&after_dataset,&after_key,&limit])
    }
    fn begin_merge(&mut self) -> Result<()> {
        self.1.stage.clear();
        self.1.stage_bytes = 0;
        // Pooled PostgreSQL sessions reuse the staging relation and its prepared plans.
        // Transactional deletion keeps repeated merges and rollback isolated.
        self.batch_execute(match self.0.backend {
            Backend::Postgis(_) => "CREATE TEMP TABLE IF NOT EXISTS gl_merge_stage(dataset text, feature_id text, before_value text, after_value text, PRIMARY KEY(dataset,feature_id)) ON COMMIT DELETE ROWS; DELETE FROM gl_merge_stage",
            Backend::Sqlite(_) => "DROP TABLE IF EXISTS gl_merge_stage; CREATE TEMP TABLE gl_merge_stage(dataset text, feature_id text, before_value text, after_value text, PRIMARY KEY(dataset,feature_id))",
        })
    }
    fn merge_page(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        current: i64,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<Vec<Row>> {
        self.query("WITH batch AS MATERIALIZED (SELECT project,dataset,feature_id,properties,geometry_json,resolved_head,resolution_stale FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($5,$6) ORDER BY dataset,feature_id LIMIT 32) SELECT c.dataset,c.feature_id,c.properties,c.geometry_json,c.resolved_head,c.resolution_stale,b.properties,b.geometry_json,o.properties,o.geometry_json FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) LEFT JOIN gl_history o ON o.project=c.project AND o.dataset=c.dataset AND o.feature_id=c.feature_id AND o.valid_from<=$4 AND (o.valid_to IS NULL OR o.valid_to>$4) ORDER BY c.dataset,c.feature_id", &[&project, &workspace, &base, &current, &after_dataset, &after_key])
    }
    fn stage_merge(
        &mut self,
        dataset: &str,
        key: &str,
        before: &Option<String>,
        after: &Option<String>,
    ) -> Result<()> {
        let bytes = dataset
            .len()
            .saturating_add(key.len())
            .saturating_add(before.as_ref().map_or(0, String::len))
            .saturating_add(after.as_ref().map_or(0, String::len));
        if self.1.stage_bytes.saturating_add(bytes) > 1024 * 1024 {
            self.flush_stage()?;
        }
        self.1.stage.push((
            dataset.to_owned(),
            key.to_owned(),
            before.clone(),
            after.clone(),
        ));
        self.1.stage_bytes = self.1.stage_bytes.saturating_add(bytes);
        if self.1.stage.len() >= 32 || self.1.stage_bytes >= 1024 * 1024 {
            self.flush_stage()?;
        }
        Ok(())
    }
    fn publication_receipt(
        &mut self,
        project: &str,
        subject: &str,
        request_id: &str,
    ) -> Result<Option<Row>> {
        self.query_opt("SELECT payload,result FROM gl_idempotency WHERE project=$1 AND subject=$2 AND request_id=$3", &[&project, &subject, &request_id])
    }
    fn append_commit(
        &mut self,
        project: &str,
        revision: i64,
        workspace: &str,
        subject: &str,
        message: &str,
    ) -> Result<()> {
        self.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES($1,$2,$3,$4,$5)", &[&project, &revision, &workspace, &subject, &message])
    }
    fn append_changes(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        self.execute("INSERT INTO gl_commit_changes SELECT $1,$2,dataset,feature_id,NULLIF(before_value,'null'),after_value FROM gl_merge_stage", &[&project, &revision])
    }
    fn close_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        if matches!(self.0.backend, Backend::Postgis(_)) {
            // Pooled sessions retain the old batch's statistics after commit
            // clears the temp table. Refresh its cardinality for this batch.
            self.batch_execute("ANALYZE gl_merge_stage")?;
        }
        self.sync_sources(project)?;
        // Drive both backends from changed keys, even when a new project's
        // history is underestimated by statistics dominated by another project.
        let sql = match self.0.backend {
            Backend::Sqlite(_) => {
                "UPDATE gl_history SET valid_to=$2 WHERE rowid IN (SELECT h.rowid FROM gl_merge_stage m CROSS JOIN gl_history h INDEXED BY history_live ON h.project=$1 AND h.dataset=m.dataset AND h.feature_id=m.feature_id AND h.valid_to IS NULL)"
            }
            Backend::Postgis(_) => {
                // LIMIT keeps each live-key lookup correlated with the staged
                // row. The TID is consumed in this statement while the project
                // publication lock protects the history rows.
                "UPDATE gl_history AS h SET valid_to=$2 FROM gl_merge_stage m CROSS JOIN LATERAL (SELECT ctid FROM gl_history WHERE project=$1 AND dataset=m.dataset AND feature_id=m.feature_id AND valid_to IS NULL LIMIT 1) live WHERE h.ctid=live.ctid"
            }
        };
        self.execute(sql, &[&project, &revision])
    }
    fn append_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        self.execute(&format!("INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,{columns}) SELECT $1,dataset,feature_id,$2,gl_json_field(after_value,'properties'),gl_json_field(after_value,'geometry'),{values} FROM gl_merge_stage", columns=self.geometry_columns(), values=self.geometry_values("gl_json_field(after_value,'geometry')")), &[&project, &revision])
    }
    fn advance_head(&mut self, project: &str, revision: i64) -> Result<()> {
        self.execute(
            "UPDATE gl_projects SET head=$2 WHERE id=$1",
            &[&project, &revision],
        )
    }
    fn save_receipt(
        &mut self,
        project: &str,
        subject: &str,
        request_id: &str,
        payload: &str,
        result: &str,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO gl_idempotency VALUES($1,$2,$3,$4,$5)",
            &[&project, &subject, &request_id, &payload, &result],
        )
    }
    fn mark_resolved(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        head: i64,
    ) -> Result<()> {
        self.execute(&format!("UPDATE {} SET resolved_head=$5,resolution_stale=false WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4", self.workspace_key_table()), &[&project, &workspace, &dataset, &key, &head])
    }
    fn stage_resolution(&mut self, dataset: &str, key: &str, after: &Option<String>) -> Result<()> {
        self.execute(
            "INSERT INTO gl_merge_stage(dataset,feature_id,after_value) VALUES($1,$2,$3)",
            &[&dataset, &key, &after],
        )
    }
    fn clear_deltas(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute(
            &format!(
                "DELETE FROM {} WHERE project=$1 AND workspace=$2",
                self.workspace_key_table()
            ),
            &[&project, &workspace],
        )
    }
    fn replace_deltas_with_merge(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.flush_stage()?;
        self.execute(&format!("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geometry_json,{columns}) SELECT $1,$2,dataset,feature_id,gl_json_field(after_value,'properties'),gl_json_field(after_value,'geometry'),{values} FROM gl_merge_stage", columns=self.geometry_columns(), values=self.geometry_values("gl_json_field(after_value,'geometry')")), &[&project, &workspace])
    }
    fn advance_base(&mut self, project: &str, workspace: &str, revision: i64) -> Result<()> {
        self.execute(
            "UPDATE gl_workspaces SET base_revision=$3 WHERE project=$1 AND id=$2",
            &[&project, &workspace, &revision],
        )
    }
    fn count_commit_changes(&mut self, project: &str, revision: i64) -> Result<Row> {
        self.query_one(
            "SELECT count(*) FROM gl_commit_changes WHERE project=$1 AND revision=$2",
            &[&project, &revision],
        )
    }
    fn restore_deltas(&mut self, project: &str, revision: i64, workspace: &str) -> Result<()> {
        self.execute(&format!("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geometry_json,{columns}) SELECT project,$3,dataset,feature_id,gl_json_field(before_value,'properties'),gl_json_field(before_value,'geometry'),{values} FROM gl_commit_changes WHERE project=$1 AND revision=$2", columns=self.geometry_columns(), values=self.geometry_values("gl_json_field(before_value,'geometry')")), &[&project, &revision, &workspace])
    }
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
pub(crate) struct SqlStorage {
    pub(crate) storage: Storage,
    pub(crate) pool: std::sync::Arc<postgres::Pool>,
}
impl StorageBackend for SqlStorage {
    fn name(&self) -> &'static str {
        match self.storage {
            Storage::Sqlite(_) => "sqlite",
            Storage::Postgis(_) => "postgis",
        }
    }
    fn initialize(&self, timeout: Duration) -> Result<()> {
        Client::open(&self.storage, &self.pool, timeout)?.migrate()
    }
    fn health(&self, timeout: Duration) -> Result<()> {
        let mut c = Client::open(&self.storage, &self.pool, timeout)?;
        if c.query_one("SELECT version FROM gl_format WHERE singleton=true", &[])?
            .get::<_, i32>(0usize)?
            != FORMAT_VERSION
        {
            return Err(Error::new(409, "unsupported storage format"));
        }
        Ok(())
    }
    fn begin(&self, read_only: bool, timeout: Duration) -> Result<Box<dyn RepositoryTransaction>> {
        Ok(Box::new(
            Client::open(&self.storage, &self.pool, timeout)?.transaction(read_only)?,
        ))
    }
    fn pool_stats(&self) -> Option<crate::PoolStats> {
        matches!(self.storage, Storage::Postgis(_)).then(|| self.pool.stats())
    }
    fn export(
        &self,
        out: &mut dyn std::io::Write,
        timeout: Duration,
    ) -> Result<crate::DataSummary> {
        let missing = || Error::new(404, "no GeoLedger database to export");
        if matches!(&self.storage, Storage::Sqlite(path) if !path.is_file()) {
            return Err(missing());
        }
        let mut t = Client::open(&self.storage, &self.pool, timeout)?.transaction(true)?;
        let exists = match self.storage {
            Storage::Sqlite(_) => t
                .query_opt(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_format'",
                    &[],
                )?
                .is_some(),
            Storage::Postgis(_) => t
                .query_one("SELECT to_regclass('public.gl_format') IS NOT NULL", &[])?
                .get::<_, bool>(0usize)?,
        };
        if !exists {
            return Err(missing());
        }
        portable::export(&mut t, self.name(), out)
    }
    fn import(
        &self,
        input: &mut dyn std::io::BufRead,
        timeout: Duration,
    ) -> Result<crate::DataSummary> {
        Client::open(&self.storage, &self.pool, timeout)?.migrate()?;
        portable::import(Client::open(&self.storage, &self.pool, timeout)?, input)
    }
    fn backup(&self, target: &std::path::Path, timeout: Duration) -> Result<()> {
        let Storage::Sqlite(source) = &self.storage else {
            return Err(Error::new(
                409,
                "PostgreSQL backups use pg_dump or WAL archiving (see docs/production.md); `geoledger-server export` gives a portable logical copy",
            ));
        };
        let missing = || Error::new(404, "no GeoLedger database to back up");
        if !source.is_file() {
            return Err(missing());
        }
        let target_text = target
            .to_str()
            .ok_or_else(|| Error::new(400, "backup path must be valid UTF-8"))?
            .to_owned();
        // Reserve the target as a new private file (VACUUM INTO accepts an empty
        // file), so a backup is never readable by others and never overwrites.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(target).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Error::new(409, "backup target already exists")
            } else {
                Error::new(500, "cannot create backup file").caused_by(e)
            }
        })?;
        let result = (|| {
            let mut c = Client::open(&self.storage, &self.pool, timeout)?;
            let exists = c
                .query_opt(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_format'",
                    &[],
                )?
                .is_some();
            if !exists {
                return Err(missing());
            }
            // VACUUM INTO reads one consistent snapshot while writers continue (WAL).
            c.execute("VACUUM INTO $1", &[&target_text])?;
            drop(c);
            verify_sqlite_file(target, timeout)
        })();
        if result.is_err() {
            // Never leave a file that looks like a usable backup.
            let _ = std::fs::remove_file(target);
        }
        result
    }
}
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
        reader.commit()?;
        commit_tx.send(()).unwrap();
        worker.join().unwrap()?;
        let mut reader = Client::open(&storage, &pool, timeout)?.transaction(true)?;
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
            assert_eq!(
        t.query_one(
            "SELECT count(*) FROM gl_history WHERE feature_id='00001001' AND valid_to IS NULL",
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
            let statements=t.query("SELECT name,generic_plans,custom_plans FROM pg_prepared_statements WHERE statement LIKE 'WITH base AS MATERIALIZED%'",&[])?;
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
                plan.contains("history_live"),
                "current-head query missed partial index: {plan}"
            );
            assert!(
                !plan.contains("Rows Removed by Filter: 31968"),
                "old versions still scanned: {plan}"
            );
            eprintln!(
                "hot_bbox_executions=10 generic_plans=0 current_index=history_live logical_features=64 versions_each=1000 page_rows=32\n{plan}"
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
        fn postgis_hot_spatial_queries_publication_looks_up_changed_history() -> TestResult {
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
            }
            t.batch_execute("SET LOCAL plan_cache_mode=force_custom_plan")?;
            t.begin_merge()?;
            t.stage_merge("d", "00000001", &Some("{}".into()), &None)?;
            t.stage_merge("d", "00000002", &Some("{}".into()), &Some("{}".into()))?;
            for n in 0..8 {
                t.stage_merge("d", &format!("new-{n}"), &None, &Some("{}".into()))?;
            }
            t.close_history(&project, 2)?;
            let sql: String = t.query_one("SELECT statement FROM pg_prepared_statements WHERE statement LIKE 'UPDATE gl_history%SET valid_to%'", &[])?.get(0usize)?;
            for mode in ["force_custom_plan", "force_generic_plan"] {
                t.batch_execute(&format!("SET LOCAL plan_cache_mode={mode}"))?;
                let plan = t
                    .query(
                        &format!("EXPLAIN (ANALYZE, COSTS OFF) {sql}"),
                        &[&project, &2_i64],
                    )?
                    .iter()
                    .map(|r| r.get::<_, String>(0usize))
                    .collect::<Result<Vec<_>>>()?
                    .join("\n");
                assert!(
                    plan.contains("Index Scan using history_live"),
                    "publication lost live-key lookup ({mode}): {plan}"
                );
                assert!(
                    plan.lines().any(|line| line.contains("Index Cond:")
                        && line.contains("project")
                        && line.contains("dataset")
                        && line.contains("feature_id")),
                    "publication scanned a project prefix ({mode}): {plan}"
                );
                assert!(
                    !plan.contains("Seq Scan on gl_history"),
                    "small publication scanned history: {plan}"
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
                t.query_one(
                    "SELECT count(*) FROM gl_history WHERE project=$1 AND valid_to IS NULL",
                    &[&project]
                )?
                .get::<_, i64>(0usize)?,
                9998
            );
            Ok(())
        }
    }
}
