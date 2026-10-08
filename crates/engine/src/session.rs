//! Portable transaction boundary; SQL is shared, locking and spatial indexes are explicit dialect choices.
pub(crate) mod pgtls;
pub(crate) mod postgres;
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
    Sqlite(rusqlite::Connection),
    Postgis(postgres::Client),
}
pub(crate) struct Client {
    backend: Backend,
    deadline: Instant,
    transaction: bool,
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
                c.busy_timeout(timeout).map_err(sqlite_error)?;
                c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")
                    .map_err(sqlite_error)?;
                c.progress_handler(1000, Some(move || Instant::now() >= deadline));
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
                c.create_scalar_function("gl_intersects", 5, flags, |ctx| {
                    let source: Option<String> = ctx.get(0usize)?;
                    match source {
                        None => Ok(false),
                        Some(s) => crate::geometry::intersects(
                            &s,
                            [
                                ctx.get(1usize)?,
                                ctx.get(2usize)?,
                                ctx.get(3usize)?,
                                ctx.get(4usize)?,
                            ],
                        )
                        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e))),
                    }
                })
                .map_err(sqlite_error)?;
                c.create_scalar_function("gl_bound", 2, flags, |ctx| {
                    let source: Option<String> = ctx.get(0usize)?;
                    let axis: usize = ctx.get(1usize)?;
                    let Some(s) = source else {
                        return Ok(None::<f64>);
                    };
                    Ok(crate::geometry::bounds(&s)
                        .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?
                        .map(|r| [r.min().x, r.min().y, r.max().x, r.max().y][axis.min(3)]))
                })
                .map_err(sqlite_error)?;
                Backend::Sqlite(c)
            }
        };
        Ok(Self {
            backend,
            deadline,
            transaction: false,
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
                            Cell::Null => Box::new(None::<String>),
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
                c.busy_timeout(self.deadline.saturating_duration_since(Instant::now()))
                    .map_err(sqlite_error)?;
                c.execute_batch(sql).map_err(sqlite_error)
            }
            Backend::Postgis(c) => c.batch_execute(sql),
        }
    }
    pub fn transaction(mut self, read_only: bool) -> Result<SqlTransaction> {
        if read_only && matches!(self.backend, Backend::Sqlite(_)) {
            self.batch_execute("PRAGMA query_only=ON")?;
        }
        self.batch_execute(match self.backend {
            Backend::Sqlite(_) if !read_only => "BEGIN IMMEDIATE",
            Backend::Postgis(_) if read_only => "BEGIN ISOLATION LEVEL REPEATABLE READ",
            _ => "BEGIN",
        })?;
        self.transaction = true;
        Ok(SqlTransaction(self))
    }
    pub fn migrate(mut self) -> Result<()> {
        let sqlite = matches!(self.backend, Backend::Sqlite(_));
        if sqlite {
            self.batch_execute("PRAGMA journal_mode=WAL;")?;
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
        if exists {
            if t.query_one("SELECT version FROM gl_format WHERE singleton=true", &[])?
                .get::<_, i32>(0usize)?
                != FORMAT_VERSION
            {
                return Err(Error::new(
                    409,
                    "unsupported storage format; initialize a fresh database",
                ));
            }
        } else {
            t.batch_execute(if sqlite {
                include_str!("sqlite.sql")
            } else {
                include_str!("postgis.sql")
            })?;
        }
        t.commit()
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        if self.transaction {
            match &mut self.backend {
                Backend::Sqlite(c) => {
                    c.progress_handler(0, None::<fn() -> bool>);
                    let _ = c.execute_batch("ROLLBACK");
                }
                Backend::Postgis(c) => {
                    c.invalidate();
                }
            }
        }
    }
}
pub(crate) struct SqlTransaction(Client);
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
    pub(crate) fn spatial_filter(&self, alias: &str, table: &str, enabled: bool) -> String {
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
            Backend::Postgis(_) => format!(
                "$7 AND ST_Intersects(ST_GeomFromGeoJSON({a}geom),ST_MakeEnvelope($8,$9,$10,$11,4326))"
            ),
            Backend::Sqlite(_) => format!(
                "$7 AND {a}rowid IN (SELECT id FROM gl_{table}_spatial WHERE min_x<=$10 AND max_x>=$8 AND min_y<=$11 AND max_y>=$9) AND gl_intersects({a}geom,$8,$9,$10,$11)"
            ),
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

impl crate::repository::RepositoryTransaction for SqlTransaction {
    fn member_role(&mut self, project: &str, subject: &str) -> Result<Option<Row>> {
        self.query_opt(
            &self.lock_sql(
                "SELECT role FROM gl_project_members WHERE project=$1 AND subject=$2",
                "FOR SHARE",
            ),
            &[&project, &subject],
        )
    }
    fn project_head(&mut self, project: &str, lock: bool) -> Result<Option<Row>> {
        let sql = "SELECT head FROM gl_projects WHERE id=$1";
        self.query_opt(
            &self.lock_sql(sql, if lock { "FOR NO KEY UPDATE" } else { "" }),
            &[&project],
        )
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
        let bbox = r.bbox.unwrap_or([-180., -90., 180., 90.]);
        // Bound both UNION branches before geometry/JSON expansion and final
        // sorting. A top-level LIMIT alone let PostgreSQL scan and serialize
        // the entire remaining dataset for each 32-row page.
        let key_filter = if r.feature_id.is_some() {
            "feature_id=$6"
        } else {
            "CAST($6 AS text) IS NULL"
        };
        let spatial_filter = self.spatial_filter("h", "history", r.bbox.is_some());
        let draft_spatial_filter = self.spatial_filter("", "workspace_changes", r.bbox.is_some());
        let sql = format!("WITH base AS MATERIALIZED (
            SELECT h.feature_id,h.properties,h.geom FROM gl_history h
            WHERE h.project=$1 AND h.dataset=$2
              AND h.valid_from<=$3 AND (h.valid_to IS NULL OR h.valid_to>$3)
              AND h.properties IS NOT NULL AND h.feature_id>$5 AND {key_filter}
              AND ({spatial_filter})
              AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c
                WHERE c.project=h.project AND c.dataset=h.dataset AND c.feature_id=h.feature_id AND c.workspace=$4)
            ORDER BY h.feature_id LIMIT $12
          ), draft AS MATERIALIZED (
            SELECT feature_id,properties,geom FROM gl_workspace_changes
            WHERE project=$1 AND dataset=$2 AND workspace=$4
              AND properties IS NOT NULL AND feature_id>$5 AND {key_filter}
              AND ({draft_spatial_filter})
            ORDER BY feature_id LIMIT $12
          ), page AS MATERIALIZED (
            SELECT * FROM base UNION ALL SELECT * FROM draft ORDER BY feature_id LIMIT $12
          ) SELECT feature_id,properties,geom FROM page ORDER BY feature_id");
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
    fn feature_at(
        &mut self,
        project: &str,
        dataset: &str,
        key: &str,
        revision: i64,
    ) -> Result<Option<Row>> {
        self.query_opt("SELECT properties,geom FROM gl_history WHERE project=$1 AND dataset=$2 AND feature_id=$3 AND valid_from<=$4 AND (valid_to IS NULL OR valid_to>$4)", &[&project, &dataset, &key, &revision])
    }
    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>> {
        self.query_opt(
            "SELECT 1 FROM gl_datasets WHERE project=$1 AND id=$2",
            &[&project, &dataset],
        )
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
            "INSERT INTO gl_project_members VALUES($1,$2,'owner')",
            &[&project, &subject],
        )
    }
    fn list_projects(&mut self, subject: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT p.id,p.name,p.head FROM gl_projects p JOIN gl_project_members m ON m.project=p.id WHERE m.subject=$1 AND p.id>$2 ORDER BY p.id LIMIT $3", &[&subject, &after, &limit])
    }
    fn project_info(&mut self, project: &str) -> Result<Row> {
        self.query_one("SELECT name,head FROM gl_projects WHERE id=$1", &[&project])
    }
    fn owner_summary(&mut self, project: &str, subject: &str) -> Result<Row> {
        self.query_one("SELECT count(*) FILTER (WHERE role='owner'), coalesce(max(CASE WHEN subject=$2 AND role='owner' THEN 1 ELSE 0 END),0) FROM gl_project_members WHERE project=$1", &[&project, &subject])
    }
    fn set_member(&mut self, project: &str, subject: &str, role: &str) -> Result<()> {
        self.execute("INSERT INTO gl_project_members VALUES($1,$2,$3) ON CONFLICT(project,subject) DO UPDATE SET role=excluded.role", &[&project, &subject, &role])
    }
    fn insert_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()> {
        self.execute(
            "INSERT INTO gl_datasets VALUES($1,$2,$3)",
            &[&project, &dataset, &name],
        )
    }
    fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query(
            "SELECT id,name FROM gl_datasets WHERE project=$1 AND id>$2 ORDER BY id LIMIT $3",
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
    fn put_delta(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        properties: &Option<String>,
        geometry: &Option<&str>,
    ) -> Result<()> {
        self.execute("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geom) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(project,workspace,dataset,feature_id) DO UPDATE SET properties=excluded.properties,geom=excluded.geom", &[&project, &workspace, &dataset, &key, &properties, &geometry])
    }
    fn invalidate_resolutions(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute("UPDATE gl_workspace_changes SET resolution_stale=true WHERE project=$1 AND workspace=$2 AND resolved_head IS NOT NULL", &[&project, &workspace])
    }
    fn has_resolution(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
    ) -> Result<Row> {
        self.query_one("SELECT EXISTS(SELECT 1 FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4 AND resolved_head IS NOT NULL)", &[&project, &workspace, &dataset, &key])
    }
    fn remove_delta(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
    ) -> Result<()> {
        self.execute("DELETE FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4", &[&project, &workspace, &dataset, &key])
    }
    fn count_deltas(&mut self, project: &str, workspace: &str) -> Result<Row> {
        self.query_one(
            "SELECT count(*) FROM gl_workspace_changes WHERE project=$1 AND workspace=$2",
            &[&project, &workspace],
        )
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
        self.query("WITH batch AS MATERIALIZED (SELECT * FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($4,$5) ORDER BY dataset,feature_id LIMIT $6) SELECT c.dataset,c.feature_id,c.properties,c.geom,b.properties,b.geom FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) ORDER BY c.dataset,c.feature_id", &[&project, &workspace, &base, &after_dataset, &after_key, &limit])
    }
    fn history_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT revision,subject,message,created_at FROM gl_commits WHERE project=$1 AND revision>$2 ORDER BY revision LIMIT $3", &[&project, &after, &limit])
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
    fn begin_merge(&mut self) -> Result<()> {
        self.batch_execute("DROP TABLE IF EXISTS center_merge; CREATE TEMP TABLE center_merge(dataset text, feature_id text, before_value text, after_value text)")
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
        self.query("WITH batch AS MATERIALIZED (SELECT * FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($5,$6) ORDER BY dataset,feature_id LIMIT 32) SELECT c.dataset,c.feature_id,c.properties,c.geom,c.resolved_head,c.resolution_stale,b.properties,b.geom,o.properties,o.geom FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) LEFT JOIN gl_history o ON o.project=c.project AND o.dataset=c.dataset AND o.feature_id=c.feature_id AND o.valid_from<=$4 AND (o.valid_to IS NULL OR o.valid_to>$4) ORDER BY c.dataset,c.feature_id", &[&project, &workspace, &base, &current, &after_dataset, &after_key])
    }
    fn stage_merge(
        &mut self,
        dataset: &str,
        key: &str,
        before: &Option<String>,
        after: &Option<String>,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO center_merge VALUES($1,$2,$3,$4)",
            &[&dataset, &key, &before, &after],
        )
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
        self.execute("INSERT INTO gl_commit_changes SELECT $1,$2,dataset,feature_id,NULLIF(before_value,'null'),after_value FROM center_merge", &[&project, &revision])
    }
    fn close_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.execute("UPDATE gl_history AS h SET valid_to=$2 FROM center_merge m WHERE h.project=$1 AND h.dataset=m.dataset AND h.feature_id=m.feature_id AND h.valid_to IS NULL", &[&project, &revision])
    }
    fn append_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.execute("INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geom) SELECT $1,dataset,feature_id,$2,gl_json_field(after_value,'properties'),gl_json_field(after_value,'geometry') FROM center_merge", &[&project, &revision])
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
        self.execute("UPDATE gl_workspace_changes SET resolved_head=$5,resolution_stale=false WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4", &[&project, &workspace, &dataset, &key, &head])
    }
    fn stage_resolution(&mut self, dataset: &str, key: &str, after: &Option<String>) -> Result<()> {
        self.execute(
            "INSERT INTO center_merge(dataset,feature_id,after_value) VALUES($1,$2,$3)",
            &[&dataset, &key, &after],
        )
    }
    fn clear_deltas(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute(
            "DELETE FROM gl_workspace_changes WHERE project=$1 AND workspace=$2",
            &[&project, &workspace],
        )
    }
    fn replace_deltas_with_merge(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geom) SELECT $1,$2,dataset,feature_id,gl_json_field(after_value,'properties'),gl_json_field(after_value,'geometry') FROM center_merge", &[&project, &workspace])
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
        self.execute("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geom) SELECT project,$3,dataset,feature_id,gl_json_field(before_value,'properties'),gl_json_field(before_value,'geometry') FROM gl_commit_changes WHERE project=$1 AND revision=$2", &[&project, &revision, &workspace])
    }
    fn commit(self: Box<Self>) -> Result<()> {
        SqlTransaction::commit(*self)
    }
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_json_field_preserves_raw_json_and_unquotes_strings() -> Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let mut client = Client::open(
            &Storage::Sqlite(dir.path().join("json.db")),
            &Arc::new(postgres::Pool::default()),
            Duration::from_secs(5),
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
}
