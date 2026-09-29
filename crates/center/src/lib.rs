#![forbid(unsafe_code)]

mod http;
use geoledger_core::{Cell, Record, merge::merge_record};
pub use http::{Tokens, router};
use postgres::{Client, Row, Transaction};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use uuid::Uuid;

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct Error {
    pub status: u16,
    pub body: Value,
}
impl Error {
    fn new(status: u16, message: &str) -> Self {
        Self {
            status,
            body: json!({"error":message}),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.body)
    }
}
impl std::error::Error for Error {}
impl From<postgres::Error> for Error {
    fn from(_: postgres::Error) -> Self {
        Self::new(503, "database operation failed")
    }
}
fn bad() -> Error {
    Error::new(400, "invalid request")
}
fn missing() -> Error {
    Error::new(404, "resource unavailable")
}
fn stale() -> Error {
    Error::new(409, "stale version or workspace not open")
}
fn decode<T: DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|_| bad())
}
fn text(s: &str, max: usize) -> Result<()> {
    if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
        Err(bad())
    } else {
        Ok(())
    }
}
fn id(s: &str) -> Result<()> {
    Uuid::parse_str(s).map(|_| ()).map_err(|_| bad())
}

/// Independent central application. Each operation owns one connection and transaction.
/// Configuration deliberately has no Debug implementation: it contains a DSN.
#[derive(Clone)]
pub struct CenterApplication {
    dsn: String,
}
impl CenterApplication {
    pub fn new(dsn: String) -> Self {
        Self { dsn }
    }
    fn connect(&self) -> Result<Client> {
        let mut config: postgres::Config = self
            .dsn
            .parse()
            .map_err(|_| Error::new(503, "invalid database configuration"))?;
        config.connect_timeout(Duration::from_secs(10));
        let tls = native_tls::TlsConnector::new()
            .map_err(|_| Error::new(503, "TLS configuration failed"))?;
        let mut c = config.connect(postgres_native_tls::MakeTlsConnector::new(tls))?;
        c.batch_execute("SET statement_timeout='30s'; SET lock_timeout='10s'; SET search_path=pg_catalog,public")?;
        Ok(c)
    }
    pub fn migrate(&self) -> Result<()> {
        let mut c = self.connect()?;
        let mut t = c.transaction()?;
        // Serializes only bootstrap, not draft operations or publication.
        t.query_one("SELECT pg_advisory_xact_lock(7881,1)", &[])?;
        let exists: bool = t
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='_geoledger_center')",
                &[],
            )?
            .get(0);
        if exists {
            let version: i32 = t
                .query_one(
                    "SELECT version FROM _geoledger_center.format WHERE singleton",
                    &[],
                )?
                .get(0);
            if version != 1 {
                return Err(Error::new(409, "unsupported center schema format"));
            }
        } else {
            t.batch_execute(include_str!("schema.sql"))?;
        }
        t.commit()?;
        Ok(())
    }
    pub fn check_schema(&self) -> Result<()> {
        let mut c = self.connect()?;
        let version: i32 = c
            .query_one(
                "SELECT version FROM _geoledger_center.format WHERE singleton",
                &[],
            )?
            .get(0);
        if version != 1 {
            return Err(Error::new(409, "center schema format mismatch"));
        }
        Ok(())
    }

    /// Test harness guard; checks the database name before bootstrap or any writes.
    pub fn check_test_database(&self) -> Result<()> {
        let name: String = self
            .connect()?
            .query_one("SELECT current_database()", &[])?
            .get(0);
        if name != "geoledger_test" {
            return Err(Error::new(400, "tests require geoledger_test"));
        }
        Ok(())
    }
    pub fn execute(&self, subject: &str, operation: &str, input: Value) -> Result<Value> {
        text(subject, 128)?;
        if input.to_string().len() > MAX_BYTES {
            return Err(Error::new(413, "request too large"));
        }
        let command: Command = decode(json!({"operation":operation,"input":input}))?;
        let mut c = self.connect()?;
        let mut t = c.transaction()?;
        let format: i32 = t
            .query_one(
                "SELECT version FROM _geoledger_center.format WHERE singleton",
                &[],
            )?
            .get(0);
        if format != 1 {
            return Err(Error::new(409, "unsupported center schema format"));
        }
        let result = match command {
            Command::CreateProject(r) => create_project(&mut t, subject, r),
            Command::ListProjects(r) => list_projects(&mut t, subject, r),
            Command::GetProject(r) => get_project(&mut t, subject, r),
            Command::SetMember(r) => set_member(&mut t, subject, r),
            Command::CreateDataset(r) => create_dataset(&mut t, subject, r),
            Command::ListDatasets(r) => list_datasets(&mut t, subject, r),
            Command::CreateWorkspace(r) => create_workspace(&mut t, subject, r),
            Command::ListWorkspaces(r) => list_workspaces(&mut t, subject, r),
            Command::GetWorkspace(r) => get_workspace(&mut t, subject, r),
            Command::Save(r) => save(&mut t, subject, r),
            Command::Discard(r) => discard(&mut t, subject, r),
            Command::Features(r) => features(&mut t, subject, r),
            Command::Diff(r) => diff(&mut t, subject, r),
            Command::Conflicts(r) => list_conflicts(&mut t, subject, r),
            Command::Resolve(r) => resolve(&mut t, subject, r),
            Command::History(r) => history(&mut t, subject, r),
            Command::Commit(r) => commit_detail(&mut t, subject, r),
            Command::Publish(r) => publish(&mut t, subject, r),
            Command::Rebase(r) => rebase(&mut t, subject, r),
            Command::Restore(r) => restore(&mut t, subject, r),
        }?;
        if result.to_string().len() > MAX_BYTES {
            return Err(Error::new(413, "response too large; use a smaller page"));
        }
        t.commit()?;
        Ok(result)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Name {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    project: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedProject {
    project: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    project: String,
    subject: String,
    role: String,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default)]
    after: String,
    #[serde(default = "page_size")]
    limit: i64,
}
fn page_size() -> i64 {
    100
}
impl Page {
    fn check(&self) -> Result<()> {
        if !(1..=1000).contains(&self.limit) || self.after.len() > 512 {
            Err(bad())
        } else {
            Ok(())
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectPage {
    project: String,
    #[serde(default)]
    after: String,
    #[serde(default = "page_size")]
    limit: i64,
}
impl ProjectPage {
    fn page(&self) -> Result<()> {
        Page {
            after: self.after.clone(),
            limit: self.limit,
        }
        .check()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Workspace {
    project: String,
    workspace: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    project: String,
    workspace: String,
    expected_workspace_version: i64,
}
fn membership(t: &mut Transaction<'_>, p: &str, s: &str, write: bool) -> Result<String> {
    id(p)?;
    let row = t.query_opt("SELECT role FROM _geoledger_center.project_members WHERE project=$1::text::uuid AND subject=$2 FOR SHARE", &[&p,&s])?.ok_or_else(missing)?;
    let role: String = row.get(0);
    if write && role == "viewer" {
        return Err(missing());
    }
    Ok(role)
}
fn audit(t: &mut Transaction<'_>, p: &str, s: &str, action: &str, detail: Value) -> Result<()> {
    t.execute("INSERT INTO _geoledger_center.audit_events(project,subject,action,detail) VALUES($1::text::uuid,$2,$3,$4::text::jsonb)",&[&p,&s,&action,&detail.to_string()])?;
    Ok(())
}
fn head(t: &mut Transaction<'_>, p: &str, lock: bool) -> Result<i64> {
    id(p)?;
    let sql = if lock {
        "SELECT head FROM _geoledger_center.projects WHERE id=$1::text::uuid FOR NO KEY UPDATE"
    } else {
        "SELECT head FROM _geoledger_center.projects WHERE id=$1::text::uuid"
    };
    Ok(t.query_opt(sql, &[&p])?.ok_or_else(missing)?.get(0))
}
fn workspace(
    t: &mut Transaction<'_>,
    s: &str,
    p: &str,
    w: &str,
    version: Option<i64>,
) -> Result<(i64, i64, String)> {
    membership(t, p, s, version.is_some())?;
    id(w)?;
    let sql = if version.is_some() {
        "SELECT base_revision,version,status FROM _geoledger_center.workspaces WHERE project=$1::text::uuid AND id=$2::text::uuid AND owner=$3 FOR UPDATE"
    } else {
        "SELECT base_revision,version,status FROM _geoledger_center.workspaces WHERE project=$1::text::uuid AND id=$2::text::uuid AND owner=$3 FOR SHARE"
    };
    let r = t.query_opt(sql, &[&p, &w, &s])?.ok_or_else(missing)?;
    let v: i64 = r.get(1);
    let status: String = r.get(2);
    if version.is_some_and(|x| x != v || status != "open") {
        return Err(stale());
    }
    Ok((r.get(0), v, status))
}
fn bump(t: &mut Transaction<'_>, p: &str, w: &str, status: &str) -> Result<i64> {
    Ok(t.query_one("UPDATE _geoledger_center.workspaces SET version=version+1,status=$3 WHERE project=$1::text::uuid AND id=$2::text::uuid RETURNING version",&[&p,&w,&status])?.get(0))
}
fn create_project(t: &mut Transaction<'_>, s: &str, r: Name) -> Result<Value> {
    text(&r.name, 256)?;
    let p = Uuid::new_v4().to_string();
    t.execute(
        "INSERT INTO _geoledger_center.projects(id,name) VALUES($1::text::uuid,$2)",
        &[&p, &r.name],
    )?;
    t.execute(
        "INSERT INTO _geoledger_center.project_members VALUES($1::text::uuid,$2,'owner')",
        &[&p, &s],
    )?;
    audit(t, &p, s, "create_project", json!({}))?;
    Ok(json!({"project":p,"name":r.name,"head":0}))
}
fn list_projects(t: &mut Transaction<'_>, s: &str, r: Page) -> Result<Value> {
    r.check()?;
    let rows=t.query("SELECT p.id::text,p.name,p.head FROM _geoledger_center.projects p JOIN _geoledger_center.project_members m ON m.project=p.id WHERE m.subject=$1 AND p.id::text>$2 ORDER BY p.id::text LIMIT $3",&[&s,&r.after,&r.limit])?;
    Ok(json!(rows.iter().map(|x|json!({"project":x.get::<_,String>(0),"name":x.get::<_,String>(1),"head":x.get::<_,i64>(2)})).collect::<Vec<_>>()))
}
fn get_project(t: &mut Transaction<'_>, s: &str, r: Project) -> Result<Value> {
    let role = membership(t, &r.project, s, false)?;
    let row = t.query_one(
        "SELECT name,head FROM _geoledger_center.projects WHERE id=$1::text::uuid",
        &[&r.project],
    )?;
    Ok(
        json!({"project":r.project,"name":row.get::<_,String>(0),"head":row.get::<_,i64>(1),"role":role}),
    )
}
fn set_member(t: &mut Transaction<'_>, s: &str, r: Member) -> Result<Value> {
    // Serialize membership management with publication; consistent lock order.
    head(t, &r.project, true)?;
    if membership(t, &r.project, s, true)? != "owner" {
        return Err(missing());
    }
    text(&r.subject, 128)?;
    if !["owner", "editor", "viewer"].contains(&r.role.as_str()) {
        return Err(bad());
    }
    let owners=t.query_one("SELECT count(*) FILTER (WHERE role='owner'), coalesce(bool_or(subject=$2 AND role='owner'),false) FROM _geoledger_center.project_members WHERE project=$1::text::uuid",&[&r.project,&r.subject])?;
    if r.role != "owner" && owners.get::<_, bool>(1) && owners.get::<_, i64>(0) == 1 {
        return Err(Error::new(409, "project requires an owner"));
    }
    t.execute("INSERT INTO _geoledger_center.project_members VALUES($1::text::uuid,$2,$3) ON CONFLICT(project,subject) DO UPDATE SET role=excluded.role",&[&r.project,&r.subject,&r.role])?;
    audit(
        t,
        &r.project,
        s,
        "set_member",
        json!({"subject":r.subject,"role":r.role}),
    )?;
    Ok(json!({"ok":true}))
}
fn create_dataset(t: &mut Transaction<'_>, s: &str, r: NamedProject) -> Result<Value> {
    membership(t, &r.project, s, true)?;
    text(&r.name, 256)?;
    let d = Uuid::new_v4().to_string();
    t.execute(
        "INSERT INTO _geoledger_center.datasets VALUES($1::text::uuid,$2::text::uuid,$3)",
        &[&r.project, &d, &r.name],
    )?;
    audit(t, &r.project, s, "create_dataset", json!({"dataset":d}))?;
    Ok(json!({"dataset":d,"name":r.name}))
}
fn list_datasets(t: &mut Transaction<'_>, s: &str, r: ProjectPage) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    r.page()?;
    let rows=t.query("SELECT id::text,name FROM _geoledger_center.datasets WHERE project=$1::text::uuid AND id::text>$2 ORDER BY id::text LIMIT $3",&[&r.project,&r.after,&r.limit])?;
    Ok(json!(
        rows.iter()
            .map(|x| json!({"dataset":x.get::<_,String>(0),"name":x.get::<_,String>(1)}))
            .collect::<Vec<_>>()
    ))
}
fn create_workspace(t: &mut Transaction<'_>, s: &str, r: Project) -> Result<Value> {
    membership(t, &r.project, s, true)?;
    let base = head(t, &r.project, false)?;
    new_workspace(t, s, &r.project, base)
}
fn new_workspace(t: &mut Transaction<'_>, s: &str, p: &str, base: i64) -> Result<Value> {
    let w = Uuid::new_v4().to_string();
    t.execute("INSERT INTO _geoledger_center.workspaces(project,id,owner,base_revision) VALUES($1::text::uuid,$2::text::uuid,$3,$4)",&[&p,&w,&s,&base])?;
    audit(
        t,
        p,
        s,
        "create_workspace",
        json!({"workspace":w,"base_revision":base}),
    )?;
    Ok(json!({"workspace":w,"base_revision":base,"version":0,"status":"open"}))
}
fn get_workspace(t: &mut Transaction<'_>, s: &str, r: Workspace) -> Result<Value> {
    let (b, v, status) = workspace(t, s, &r.project, &r.workspace, None)?;
    Ok(json!({"workspace":r.workspace,"base_revision":b,"version":v,"status":status}))
}
fn list_workspaces(t: &mut Transaction<'_>, s: &str, r: ProjectPage) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    r.page()?;
    let rows=t.query("SELECT id::text,base_revision,version,status FROM _geoledger_center.workspaces WHERE project=$1::text::uuid AND owner=$2 AND id::text>$3 ORDER BY id::text LIMIT $4",&[&r.project,&s,&r.after,&r.limit])?;
    Ok(json!(rows.iter().map(|x|json!({"workspace":x.get::<_,String>(0),"base_revision":x.get::<_,i64>(1),"version":x.get::<_,i64>(2),"status":x.get::<_,String>(3)})).collect::<Vec<_>>()))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub properties: Map<String, Value>,
    pub geometry: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Stored {
    properties: Map<String, Value>,
    geometry: Option<String>,
}
impl Stored {
    fn record(&self, key: &str) -> Record {
        let mut fields: BTreeMap<String, Cell> = self
            .properties
            .iter()
            .map(|(k, v)| {
                (
                    format!("/properties/{}", k.replace('~', "~0").replace('/', "~1")),
                    Cell::Text(v.to_string()),
                )
            })
            .collect();
        fields.insert(
            "/geometry".into(),
            self.geometry
                .clone()
                .map(Cell::Geometry)
                .unwrap_or(Cell::Null),
        );
        Record {
            key: key.into(),
            fields,
        }
    }
    fn from_record(r: Record) -> Result<Self> {
        let mut properties = Map::new();
        let mut geometry = None;
        for (k, v) in r.fields {
            if k == "/geometry" {
                geometry = match v {
                    Cell::Null => None,
                    Cell::Geometry(s) => Some(s),
                    _ => return Err(bad()),
                };
            } else {
                let key = k
                    .strip_prefix("/properties/")
                    .ok_or_else(bad)?
                    .replace("~1", "/")
                    .replace("~0", "~");
                let Cell::Text(s) = v else { return Err(bad()) };
                properties.insert(key, serde_json::from_str(&s).map_err(|_| bad())?);
            }
        }
        Ok(Self {
            properties,
            geometry,
        })
    }
}
fn geometry_shape(v: &Value) -> Result<()> {
    let obj = v.as_object().ok_or_else(bad)?;
    let kind = obj.get("type").and_then(Value::as_str).ok_or_else(bad)?;
    if kind == "GeometryCollection" {
        if obj.len() != 2 {
            return Err(bad());
        }
        for g in obj
            .get("geometries")
            .and_then(Value::as_array)
            .ok_or_else(bad)?
        {
            geometry_shape(g)?;
        }
    } else {
        if ![
            "Point",
            "MultiPoint",
            "LineString",
            "MultiLineString",
            "Polygon",
            "MultiPolygon",
        ]
        .contains(&kind)
            || obj.len() != 2
        {
            return Err(bad());
        }
        coordinates(obj.get("coordinates").ok_or_else(bad)?)?;
    }
    Ok(())
}
fn coordinates(v: &Value) -> Result<()> {
    let a = v.as_array().ok_or_else(bad)?;
    if a.first().is_some_and(Value::is_number) {
        if !(2..=3).contains(&a.len()) || a.iter().any(|x| !x.as_f64().is_some_and(f64::is_finite))
        {
            return Err(bad());
        }
    } else {
        for x in a {
            coordinates(x)?;
        }
    }
    Ok(())
}
fn normalize(t: &mut Transaction<'_>, f: Feature, key: &str) -> Result<Stored> {
    text(key, 256)?;
    if f.kind != "Feature"
        || f.id != key
        || f.properties.len() > 256
        || serde_json::to_vec(&f).map_err(|_| bad())?.len() > 16384
    {
        return Err(bad());
    }
    for k in f.properties.keys() {
        text(k, 256)?;
    }
    let geometry = if f.geometry.is_null() {
        None
    } else {
        geometry_shape(&f.geometry)?;
        let row=t.query_one("WITH g AS (SELECT ST_GeomFromGeoJSON($1::text) g) SELECT encode(ST_AsEWKB(g,'XDR'),'hex'), ST_IsValid(g) AND ST_NDims(g) IN (2,3) AND ST_SRID(g)=4326 AND NOT EXISTS(SELECT 1 FROM ST_DumpPoints(g) p WHERE NOT (ST_X(p.geom) BETWEEN -180 AND 180 AND ST_Y(p.geom) BETWEEN -90 AND 90)) FROM g",&[&f.geometry.to_string()]).map_err(|_|bad())?;
        if !row.get::<_, bool>(1) {
            return Err(bad());
        }
        Some(row.get(0))
    };
    Ok(Stored {
        properties: serde_json::from_str(
            &t.query_one(
                "SELECT $1::text::jsonb::text",
                &[&json!(f.properties).to_string()],
            )?
            .get::<_, String>(0),
        )
        .map_err(|_| bad())?,
        geometry,
    })
}
fn validate_candidate(t: &mut Transaction<'_>, key: &str, value: &Stored) -> Result<()> {
    // Merging individually valid property maps can exceed a Feature's bounds.
    if value.properties.len() > 256
        || serde_json::to_vec(&geojson(t, key, Some(value))?)
            .map_err(|_| bad())?
            .len()
            > 16384
    {
        return Err(Error::new(
            422,
            "merged feature exceeds feature limits; revise the draft",
        ));
    }
    Ok(())
}

fn geojson(t: &mut Transaction<'_>, key: &str, value: Option<&Stored>) -> Result<Value> {
    let Some(v) = value else {
        return Ok(Value::Null);
    };
    let geometry = match &v.geometry {
        None => Value::Null,
        Some(g) => serde_json::from_str::<Value>(
            &t.query_one(
                "SELECT ST_AsGeoJSON(ST_GeomFromEWKB(decode($1,'hex')),17,0)",
                &[g],
            )?
            .get::<_, String>(0),
        )
        .map_err(|_| bad())?,
    };
    Ok(json!({"type":"Feature","id":key,"properties":v.properties,"geometry":geometry}))
}
fn stored_row(r: &Row, properties: usize, geom: usize) -> Result<Option<Stored>> {
    let p: Option<String> = r.get(properties);
    p.map(|p| {
        Ok(Stored {
            properties: serde_json::from_str(&p).map_err(|_| bad())?,
            geometry: r.get(geom),
        })
    })
    .transpose()
}
fn at_revision(
    t: &mut Transaction<'_>,
    p: &str,
    d: &str,
    key: &str,
    revision: i64,
) -> Result<Option<Stored>> {
    t.query_opt("SELECT properties::text,encode(ST_AsEWKB(geom,'XDR'),'hex') FROM _geoledger_center.history WHERE project=$1::text::uuid AND dataset=$2::text::uuid AND feature_id=$3 AND valid_from<=$4 AND (valid_to IS NULL OR valid_to>$4)",&[&p,&d,&key,&revision])?.map(|r|stored_row(&r,0,1)).transpose().map(Option::flatten)
}
fn dataset(t: &mut Transaction<'_>, p: &str, d: &str) -> Result<()> {
    id(d)?;
    t.query_opt("SELECT 1 FROM _geoledger_center.datasets WHERE project=$1::text::uuid AND id=$2::text::uuid",&[&p,&d])?.ok_or_else(missing)?;
    Ok(())
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    dataset: String,
    feature_id: String,
    #[serde(deserialize_with = "required_feature")]
    feature: Option<Feature>,
}
fn required_feature<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Feature>, D::Error> {
    Option::<Feature>::deserialize(d)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    project: String,
    workspace: String,
    expected_workspace_version: i64,
    edits: Vec<Edit>,
}
fn put_delta(
    t: &mut Transaction<'_>,
    p: &str,
    w: &str,
    d: &str,
    key: &str,
    v: Option<&Stored>,
) -> Result<()> {
    let props = v.map(|x| json!(x.properties).to_string());
    let g = v.and_then(|x| x.geometry.as_deref());
    t.execute("INSERT INTO _geoledger_center.workspace_changes(project,workspace,dataset,feature_id,properties,geom) VALUES($1::text::uuid,$2::text::uuid,$3::text::uuid,$4,$5::text::jsonb,ST_GeomFromEWKB(decode($6::text,'hex'))) ON CONFLICT(project,workspace,dataset,feature_id) DO UPDATE SET properties=excluded.properties,geom=excluded.geom",&[&p,&w,&d,&key,&props,&g])?;
    Ok(())
}
fn save(t: &mut Transaction<'_>, s: &str, r: Save) -> Result<Value> {
    let (base, _, _) = workspace(
        t,
        s,
        &r.project,
        &r.workspace,
        Some(r.expected_workspace_version),
    )?;
    if r.edits.is_empty() || r.edits.len() > 100 {
        return Err(bad());
    }
    // Preserve the chosen value and its origin when subsequent edits invalidate it.
    // In particular an explicit deletion may equal an absent original base.
    t.execute("UPDATE _geoledger_center.workspace_changes SET resolution_stale=true WHERE project=$1::text::uuid AND workspace=$2::text::uuid AND resolved_head IS NOT NULL", &[&r.project,&r.workspace])?;
    let mut seen = BTreeSet::new();
    for e in r.edits {
        dataset(t, &r.project, &e.dataset)?;
        text(&e.feature_id, 256)?;
        if !seen.insert((e.dataset.clone(), e.feature_id.clone())) {
            return Err(bad());
        }
        let value = e
            .feature
            .map(|f| normalize(t, f, &e.feature_id))
            .transpose()?;
        let was_resolved: bool = t.query_one(
            "SELECT EXISTS(SELECT 1 FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid AND dataset=$3::text::uuid AND feature_id=$4 AND resolved_head IS NOT NULL)",
            &[&r.project, &r.workspace, &e.dataset, &e.feature_id],
        )?.get(0);
        if !was_resolved && value == at_revision(t, &r.project, &e.dataset, &e.feature_id, base)? {
            t.execute("DELETE FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid AND dataset=$3::text::uuid AND feature_id=$4",&[&r.project,&r.workspace,&e.dataset,&e.feature_id])?;
        } else {
            put_delta(
                t,
                &r.project,
                &r.workspace,
                &e.dataset,
                &e.feature_id,
                value.as_ref(),
            )?;
        }
    }
    let count:i64=t.query_one("SELECT count(*) FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid",&[&r.project,&r.workspace])?.get(0);
    if count > 1000 {
        return Err(Error::new(413, "workspace edit limit exceeded"));
    }
    let version = bump(t, &r.project, &r.workspace, "open")?;
    audit(
        t,
        &r.project,
        s,
        "save",
        json!({"workspace":r.workspace,"version":version}),
    )?;
    Ok(json!({"version":version,"changes":count}))
}
fn discard(t: &mut Transaction<'_>, s: &str, r: Version) -> Result<Value> {
    workspace(
        t,
        s,
        &r.project,
        &r.workspace,
        Some(r.expected_workspace_version),
    )?;
    let v = bump(t, &r.project, &r.workspace, "discarded")?;
    audit(
        t,
        &r.project,
        s,
        "discard",
        json!({"workspace":r.workspace}),
    )?;
    Ok(json!({"version":v,"status":"discarded"}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Features {
    project: String,
    dataset: String,
    workspace: Option<String>,
    revision: Option<i64>,
    feature_id: Option<String>,
    bbox: Option<[f64; 4]>,
    #[serde(default)]
    after: String,
    #[serde(default = "page_size")]
    limit: i64,
}
fn features(t: &mut Transaction<'_>, s: &str, r: Features) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    dataset(t, &r.project, &r.dataset)?;
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    if r.workspace.is_some() && r.revision.is_some() {
        return Err(bad());
    }
    if let Some(k) = &r.feature_id {
        text(k, 256)?;
    }
    let (revision, workspace_version) = if let Some(w) = &r.workspace {
        let (base, version, _) = workspace(t, s, &r.project, w, None)?;
        (base, Some(version))
    } else {
        let h = head(t, &r.project, false)?;
        let v = r.revision.unwrap_or(h);
        if v < 0 || v > h {
            return Err(bad());
        }
        (v, None)
    };
    let bbox = r.bbox.unwrap_or([-180., -90., 180., 90.]);
    if bbox.iter().any(|x| !x.is_finite())
        || bbox[0] > bbox[2]
        || bbox[1] > bbox[3]
        || bbox[0] < -180.
        || bbox[2] > 180.
        || bbox[1] < -90.
        || bbox[3] > 90.
    {
        return Err(bad());
    }
    // Remove ALL shadowed base rows before spatial filtering, including deletes and moves.
    let rows=t.query("WITH overlay AS (SELECT h.feature_id,h.properties,h.geom FROM _geoledger_center.history h WHERE h.project=$1::text::uuid AND h.dataset=$2::text::uuid AND h.valid_from<=$3 AND (h.valid_to IS NULL OR h.valid_to>$3) AND NOT EXISTS(SELECT 1 FROM _geoledger_center.workspace_changes c WHERE c.project=h.project AND c.dataset=h.dataset AND c.feature_id=h.feature_id AND c.workspace=$4::text::uuid) UNION ALL SELECT feature_id,properties,geom FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND dataset=$2::text::uuid AND workspace=$4::text::uuid) SELECT feature_id,properties::text,ST_AsGeoJSON(geom,17,0) FROM overlay WHERE properties IS NOT NULL AND feature_id>$5 AND ($6::text IS NULL OR feature_id=$6) AND (NOT $7 OR ST_Intersects(geom,ST_MakeEnvelope($8,$9,$10,$11,4326))) ORDER BY feature_id LIMIT $12",&[&r.project,&r.dataset,&revision,&r.workspace,&r.after,&r.feature_id,&r.bbox.is_some(),&bbox[0],&bbox[1],&bbox[2],&bbox[3],&r.limit])?;
    let mut values = Vec::new();
    for row in rows {
        let properties: Value =
            serde_json::from_str(&row.get::<_, String>(1)).map_err(|_| bad())?;
        let geometry: Value = row
            .get::<_, Option<String>>(2)
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|_| bad())?
            .unwrap_or(Value::Null);
        values.push(json!({"type":"Feature","id":row.get::<_,String>(0),"properties":properties,"geometry":geometry}));
    }
    if r.feature_id.is_some() {
        let mut feature = values.pop().ok_or_else(missing)?;
        feature["revision"] = json!(revision);
        feature["workspace_version"] = json!(workspace_version);
        return Ok(feature);
    }
    let next = values.last().and_then(|x| x.get("id")).cloned();
    Ok(
        json!({"type":"FeatureCollection","features":values,"revision":revision,"workspace_version":workspace_version,"next_after":next}),
    )
}
#[derive(Clone)]
struct Delta {
    dataset: String,
    key: String,
    value: Option<Stored>,
    resolved_head: Option<i64>,
    resolution_stale: bool,
}
fn deltas(t: &mut Transaction<'_>, p: &str, w: &str) -> Result<Vec<Delta>> {
    t.query("SELECT dataset::text,feature_id,properties::text,encode(ST_AsEWKB(geom,'XDR'),'hex'),resolved_head,resolution_stale FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid ORDER BY dataset,feature_id",&[&p,&w])?.iter().map(|r|Ok(Delta{dataset:r.get(0),key:r.get(1),value:stored_row(r,2,3)?,resolved_head:r.get(4),resolution_stale:r.get(5)})).collect()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Diff {
    project: String,
    workspace: String,
    #[serde(default)]
    after: String,
    #[serde(default = "page_size")]
    limit: i64,
}
fn diff(t: &mut Transaction<'_>, s: &str, r: Diff) -> Result<Value> {
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    let mut result = Vec::new();
    let mut changes = deltas(t, &r.project, &r.workspace)?;
    changes.sort_by(|a, b| (&a.dataset, &a.key).cmp(&(&b.dataset, &b.key)));
    for d in changes
        .into_iter()
        .filter(|d| format!("{}/{}", d.dataset, d.key) > r.after)
        .take(r.limit as usize)
    {
        let before = at_revision(t, &r.project, &d.dataset, &d.key, base)?;
        result.push(json!({"cursor":format!("{}/{}",d.dataset,d.key),"dataset":d.dataset,"feature_id":d.key,"base":geojson(t,&d.key,before.as_ref())?,"draft":geojson(t,&d.key,d.value.as_ref())?}));
    }
    Ok(json!({"base_revision":base,"version":version,"changes":result}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    project: String,
    #[serde(default)]
    after: i64,
    #[serde(default = "page_size")]
    limit: i64,
}
fn history(t: &mut Transaction<'_>, s: &str, r: History) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    if r.after < 0 || !(1..=1000).contains(&r.limit) {
        return Err(bad());
    }
    let rows=t.query("SELECT revision,subject,message,created_at::text FROM _geoledger_center.commits WHERE project=$1::text::uuid AND revision>$2 ORDER BY revision LIMIT $3",&[&r.project,&r.after,&r.limit])?;
    Ok(json!(rows.iter().map(|x|json!({"revision":x.get::<_,i64>(0),"subject":x.get::<_,String>(1),"message":x.get::<_,String>(2),"created_at":x.get::<_,String>(3)})).collect::<Vec<_>>()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Commit {
    project: String,
    revision: i64,
    #[serde(default)]
    after: String,
    #[serde(default = "page_size")]
    limit: i64,
}
fn commit_detail(t: &mut Transaction<'_>, s: &str, r: Commit) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    t.query_opt(
        "SELECT 1 FROM _geoledger_center.commits WHERE project=$1::text::uuid AND revision=$2",
        &[&r.project, &r.revision],
    )?
    .ok_or_else(missing)?;
    let rows=t.query("SELECT dataset::text,feature_id,before_value::text,after_value::text FROM _geoledger_center.commit_changes WHERE project=$1::text::uuid AND revision=$2 AND dataset::text || '/' || feature_id>$3 ORDER BY dataset::text,feature_id LIMIT $4",&[&r.project,&r.revision,&r.after,&r.limit])?;
    let mut out = Vec::new();
    for row in rows {
        let d: String = row.get(0);
        let key: String = row.get(1);
        let b: Option<Stored> = row
            .get::<_, Option<String>>(2)
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|_| bad())?;
        let a: Option<Stored> = row
            .get::<_, Option<String>>(3)
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|_| bad())?;
        out.push(json!({"cursor":format!("{d}/{key}"),"dataset":d,"feature_id":key,"before":geojson(t,&key,b.as_ref())?,"after":geojson(t,&key,a.as_ref())?}));
    }
    Ok(json!({"revision":r.revision,"changes":out}))
}

struct MergePlan {
    changes: Vec<Delta>,
    conflicts: Vec<Value>,
}
fn merge_plan(
    t: &mut Transaction<'_>,
    p: &str,
    w: &str,
    base: i64,
    current: i64,
) -> Result<MergePlan> {
    let mut plan = MergePlan {
        changes: Vec::new(),
        conflicts: Vec::new(),
    };
    for d in deltas(t, p, w)? {
        let b = at_revision(t, p, &d.dataset, &d.key, base)?;
        let o = at_revision(t, p, &d.dataset, &d.key, current)?;
        if let Some(resolved_at) = d.resolved_head {
            if resolved_at != current || d.resolution_stale {
                plan.conflicts.push(json!({
                    "cursor":format!("{}/{}",d.dataset,d.key),
                    "dataset":d.dataset,"feature_id":d.key,"fields":[],
                    "reason":"stale_resolution","resolved_against_revision":resolved_at,
                    "base":geojson(t,&d.key,b.as_ref())?,
                    "current":geojson(t,&d.key,o.as_ref())?,
                    "draft":geojson(t,&d.key,d.value.as_ref())?
                }));
                continue;
            }
            if let Some(value) = &d.value {
                validate_candidate(t, &d.key, value)?;
            }
            if d.value != o {
                plan.changes.push(d);
            }
            continue;
        }
        let br = b.as_ref().map(|x| x.record(&d.key));
        let or = o.as_ref().map(|x| x.record(&d.key));
        let tr = d.value.as_ref().map(|x| x.record(&d.key));
        match merge_record(br.as_ref(),or.as_ref(),tr.as_ref()) {
            Ok(v)=>{
                let value=v.map(Stored::from_record).transpose()?;
                if let Some(value) = &value { validate_candidate(t, &d.key, value)?; }
                if value!=o {plan.changes.push(Delta{value,..d});}
            },
            Err(fields)=>plan.conflicts.push(json!({"cursor":format!("{}/{}",d.dataset,d.key),"dataset":d.dataset,"feature_id":d.key,"fields":fields,"base":geojson(t,&d.key,b.as_ref())?,"current":geojson(t,&d.key,o.as_ref())?,"draft":geojson(t,&d.key,d.value.as_ref())?})),
        }
    }
    Ok(plan)
}
fn conflict_page(
    items: Vec<Value>,
    after: &str,
    limit: usize,
) -> (Vec<Value>, Option<Value>, bool) {
    let mut page = Vec::new();
    let mut bytes = 0;
    let mut truncated = false;
    for item in items
        .into_iter()
        .filter(|v| v["cursor"].as_str().is_some_and(|cursor| cursor > after))
    {
        let size = item.to_string().len();
        if page.len() >= limit || (!page.is_empty() && bytes + size > MAX_BYTES / 2) {
            truncated = true;
            break;
        }
        bytes += size;
        page.push(item);
    }
    let next = page.last().map(|v| v["cursor"].clone());
    (page, next, truncated)
}
fn conflicts(head: i64, version: i64, conflicts: Vec<Value>) -> Error {
    let total = conflicts.len();
    let (page, next, truncated) = conflict_page(conflicts, "", 100);
    Error {
        status: 409,
        body: json!({"error":"merge conflicts","head":head,"version":version,"conflicts":page,"total":total,"next_after":next,"truncated":truncated}),
    }
}
fn list_conflicts(t: &mut Transaction<'_>, s: &str, r: Diff) -> Result<Value> {
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    let current = head(t, &r.project, false)?;
    let plan = merge_plan(t, &r.project, &r.workspace, base, current)?;
    let total = plan.conflicts.len();
    let (items, next, truncated) = conflict_page(plan.conflicts, &r.after, r.limit as usize);
    Ok(
        json!({"head":current,"version":version,"conflicts":items,"total":total,"next_after":next,"truncated":truncated}),
    )
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    project: String,
    workspace: String,
    expected_workspace_version: i64,
    request_id: Uuid,
    message: String,
}
fn publish(t: &mut Transaction<'_>, s: &str, r: Publish) -> Result<Value> {
    text(&r.message, 2048)?;
    let current = head(t, &r.project, true)?;
    membership(t, &r.project, s, true)?;
    workspace(t, s, &r.project, &r.workspace, None)?;
    let request_id = r.request_id.to_string();
    let payload = serde_json::to_value(&r).map_err(|_| bad())?;
    if let Some(row)=t.query_opt("SELECT payload::text,result::text FROM _geoledger_center.idempotency WHERE project=$1::text::uuid AND subject=$2 AND request_id=$3::text::uuid",&[&r.project,&s,&request_id])? {
        let previous:Value=serde_json::from_str(&row.get::<_,String>(0)).map_err(|_|bad())?;
        if previous!=payload {return Err(Error::new(409,"request_id payload mismatch"));}
        return serde_json::from_str(&row.get::<_,String>(1)).map_err(|_|bad());
    }
    let (base, version, _) = workspace(
        t,
        s,
        &r.project,
        &r.workspace,
        Some(r.expected_workspace_version),
    )?;
    let plan = merge_plan(t, &r.project, &r.workspace, base, current)?;
    if !plan.conflicts.is_empty() {
        return Err(conflicts(current, version, plan.conflicts));
    }
    let revision = current.checked_add(1).ok_or_else(bad)?;
    t.execute("INSERT INTO _geoledger_center.commits(project,revision,workspace,subject,message) VALUES($1::text::uuid,$2,$3::text::uuid,$4,$5)",&[&r.project,&revision,&r.workspace,&s,&r.message])?;
    for d in &plan.changes {
        let before = at_revision(t, &r.project, &d.dataset, &d.key, current)?;
        let before_json = before.as_ref().map(|v| json!(v).to_string());
        let after_json = d.value.as_ref().map(|v| json!(v).to_string());
        t.execute("INSERT INTO _geoledger_center.commit_changes VALUES($1::text::uuid,$2,$3::text::uuid,$4,$5::text::jsonb,$6::text::jsonb)",&[&r.project,&revision,&d.dataset,&d.key,&before_json,&after_json])?;
        t.execute("UPDATE _geoledger_center.history SET valid_to=$4 WHERE project=$1::text::uuid AND dataset=$2::text::uuid AND feature_id=$3 AND valid_to IS NULL",&[&r.project,&d.dataset,&d.key,&revision])?;
        let props = d.value.as_ref().map(|v| json!(v.properties).to_string());
        let geom = d.value.as_ref().and_then(|v| v.geometry.as_deref());
        t.execute("INSERT INTO _geoledger_center.history(project,dataset,feature_id,valid_from,properties,geom) VALUES($1::text::uuid,$2::text::uuid,$3,$4,$5::text::jsonb,ST_GeomFromEWKB(decode($6::text,'hex')))",&[&r.project,&d.dataset,&d.key,&revision,&props,&geom])?;
        if d.value.is_some() {
            t.execute("INSERT INTO _geoledger_center.features VALUES($1::text::uuid,$2::text::uuid,$3,$4::text::jsonb,ST_GeomFromEWKB(decode($5::text,'hex'))) ON CONFLICT(project,dataset,feature_id) DO UPDATE SET properties=excluded.properties,geom=excluded.geom",&[&r.project,&d.dataset,&d.key,&props,&geom])?;
        } else {
            t.execute("DELETE FROM _geoledger_center.features WHERE project=$1::text::uuid AND dataset=$2::text::uuid AND feature_id=$3",&[&r.project,&d.dataset,&d.key])?;
        }
    }
    t.execute(
        "UPDATE _geoledger_center.projects SET head=$2 WHERE id=$1::text::uuid",
        &[&r.project, &revision],
    )?;
    let v = bump(t, &r.project, &r.workspace, "published")?;
    let result = json!({"revision":revision,"workspace":r.workspace,"version":v,"status":"published","changes":plan.changes.len()});
    t.execute("INSERT INTO _geoledger_center.idempotency VALUES($1::text::uuid,$2,$3::text::uuid,$4::text::jsonb,$5::text::jsonb)",&[&r.project,&s,&request_id,&payload.to_string(),&result.to_string()])?;
    audit(t, &r.project, s, "publish", result.clone())?;
    Ok(result)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rebase {
    project: String,
    workspace: String,
    expected_workspace_version: i64,
    expected_head: i64,
    resolutions: Vec<Edit>,
}
fn resolve(t: &mut Transaction<'_>, s: &str, r: Rebase) -> Result<Value> {
    let current = head(t, &r.project, true)?;
    let (base, _, _) = workspace(
        t,
        s,
        &r.project,
        &r.workspace,
        Some(r.expected_workspace_version),
    )?;
    if current != r.expected_head {
        return Err(stale());
    }
    if r.resolutions.is_empty() || r.resolutions.len() > 100 {
        return Err(bad());
    }
    let plan = merge_plan(t, &r.project, &r.workspace, base, current)?;
    let mut remaining: BTreeSet<(String, String)> = plan
        .conflicts
        .iter()
        .map(|v| {
            (
                v["dataset"].as_str().unwrap_or_default().into(),
                v["feature_id"].as_str().unwrap_or_default().into(),
            )
        })
        .collect();
    for e in r.resolutions {
        if !remaining.remove(&(e.dataset.clone(), e.feature_id.clone())) {
            return Err(Error::new(
                409,
                "resolution must identify a current conflict exactly once",
            ));
        }
        let value = e
            .feature
            .map(|f| normalize(t, f, &e.feature_id))
            .transpose()?;
        put_delta(
            t,
            &r.project,
            &r.workspace,
            &e.dataset,
            &e.feature_id,
            value.as_ref(),
        )?;
        t.execute("UPDATE _geoledger_center.workspace_changes SET resolved_head=$5,resolution_stale=false WHERE project=$1::text::uuid AND workspace=$2::text::uuid AND dataset=$3::text::uuid AND feature_id=$4",&[&r.project,&r.workspace,&e.dataset,&e.feature_id,&current])?;
    }
    let version = bump(t, &r.project, &r.workspace, "open")?;
    audit(
        t,
        &r.project,
        s,
        "resolve",
        json!({"workspace":r.workspace,"head":current,"version":version,"remaining":remaining.len()}),
    )?;
    Ok(json!({"head":current,"version":version,"remaining_conflicts":remaining.len()}))
}

fn rebase(t: &mut Transaction<'_>, s: &str, r: Rebase) -> Result<Value> {
    let current = head(t, &r.project, true)?;
    let (base, version, _) = workspace(
        t,
        s,
        &r.project,
        &r.workspace,
        Some(r.expected_workspace_version),
    )?;
    if current != r.expected_head {
        return Err(stale());
    }
    let mut plan = merge_plan(t, &r.project, &r.workspace, base, current)?;
    let mut required: BTreeSet<(String, String)> = plan
        .conflicts
        .iter()
        .map(|v| {
            (
                v["dataset"].as_str().unwrap_or_default().into(),
                v["feature_id"].as_str().unwrap_or_default().into(),
            )
        })
        .collect();
    for e in r.resolutions {
        if !required.remove(&(e.dataset.clone(), e.feature_id.clone())) {
            return Err(Error::new(409, "resolutions must match conflicts exactly"));
        }
        let value = e
            .feature
            .map(|f| normalize(t, f, &e.feature_id))
            .transpose()?;
        if value != at_revision(t, &r.project, &e.dataset, &e.feature_id, current)? {
            plan.changes.push(Delta {
                dataset: e.dataset,
                key: e.feature_id,
                value,
                resolved_head: None,
                resolution_stale: false,
            });
        }
    }
    if !required.is_empty() {
        return Err(conflicts(current, version, plan.conflicts));
    }
    t.execute("DELETE FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid",&[&r.project,&r.workspace])?;
    for d in &plan.changes {
        put_delta(
            t,
            &r.project,
            &r.workspace,
            &d.dataset,
            &d.key,
            d.value.as_ref(),
        )?;
    }
    t.execute("UPDATE _geoledger_center.workspaces SET base_revision=$3 WHERE project=$1::text::uuid AND id=$2::text::uuid",&[&r.project,&r.workspace,&current])?;
    let v = bump(t, &r.project, &r.workspace, "open")?;
    audit(
        t,
        &r.project,
        s,
        "rebase",
        json!({"workspace":r.workspace,"base_revision":current,"version":v}),
    )?;
    Ok(json!({"base_revision":current,"version":v,"changes":plan.changes.len()}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Restore {
    project: String,
    revision: i64,
}
fn restore(t: &mut Transaction<'_>, s: &str, r: Restore) -> Result<Value> {
    membership(t, &r.project, s, true)?;
    t.query_opt(
        "SELECT 1 FROM _geoledger_center.commits WHERE project=$1::text::uuid AND revision=$2",
        &[&r.project, &r.revision],
    )?
    .ok_or_else(missing)?;
    let result = new_workspace(t, s, &r.project, r.revision)?;
    let w = result["workspace"].as_str().ok_or_else(bad)?;
    let rows=t.query("SELECT dataset::text,feature_id,before_value::text FROM _geoledger_center.commit_changes WHERE project=$1::text::uuid AND revision=$2 ORDER BY dataset,feature_id LIMIT 1001",&[&r.project,&r.revision])?;
    if rows.len() > 1000 {
        return Err(Error::new(413, "restore exceeds workspace limit"));
    }
    for row in rows {
        let before: Option<Stored> = row
            .get::<_, Option<String>>(2)
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(|_| bad())?;
        put_delta(
            t,
            &r.project,
            w,
            &row.get::<_, String>(0),
            &row.get::<_, String>(1),
            before.as_ref(),
        )?;
    }
    audit(
        t,
        &r.project,
        s,
        "restore",
        json!({"workspace":w,"reverts_revision":r.revision}),
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codec_and_field_merge() -> Result<()> {
        let b = Stored {
            properties: serde_json::from_value(
                json!({"geometry":null,"a":1,"b":true,"~/x":[1,false]}),
            )
            .map_err(|_| bad())?,
            geometry: Some("abcd".into()),
        };
        assert_eq!(Stored::from_record(b.record("id"))?, b);
        let mut ours = b.clone();
        ours.properties.insert("a".into(), json!(2));
        let mut theirs = b.clone();
        theirs.properties.remove("geometry");
        theirs.properties.insert("b".into(), json!(false));
        let merged = merge_record(
            Some(&b.record("id")),
            Some(&ours.record("id")),
            Some(&theirs.record("id")),
        )
        .map_err(|_| bad())?
        .ok_or_else(bad)?;
        let merged = Stored::from_record(merged)?;
        assert_eq!(merged.properties["a"], 2);
        assert_eq!(merged.properties["b"], false);
        assert!(!merged.properties.contains_key("geometry"));
        theirs.properties.insert("a".into(), json!(3));
        assert_eq!(
            merge_record(
                Some(&b.record("id")),
                Some(&ours.record("id")),
                Some(&theirs.record("id"))
            )
            .err(),
            Some(vec!["/properties/a".into()])
        );
        assert!(merge_record(Some(&b.record("id")), None, Some(&theirs.record("id"))).is_err());
        Ok(())
    }
    #[test]
    fn validates_geometry_shape_and_identity() {
        assert!(decode::<Edit>(json!({"dataset":"d","feature_id":"f"})).is_err());
        assert!(decode::<Edit>(json!({"dataset":"d","feature_id":"f","feature":null})).is_ok());
        assert!(geometry_shape(&json!({"type":"Point","coordinates":[1,2]})).is_ok());
        for g in [
            json!({"type":"Point","coordinates":[1,2,3,4]}),
            json!({"type":"Point","coordinates":["NaN",2]}),
            json!({"type":"Point","coordinates":[1,2],"crs":{}}),
        ] {
            assert!(geometry_shape(&g).is_err());
        }
        assert!(decode::<Publish>(json!({"project":"p","workspace":"w","expected_workspace_version":0,"request_id":Uuid::new_v4(),"message":"x","author":"forged"})).is_err());
    }
}

#[derive(Deserialize)]
#[serde(tag = "operation", content = "input", rename_all = "snake_case")]
enum Command {
    CreateProject(Name),
    ListProjects(Page),
    GetProject(Project),
    SetMember(Member),
    CreateDataset(NamedProject),
    ListDatasets(ProjectPage),
    CreateWorkspace(Project),
    ListWorkspaces(ProjectPage),
    GetWorkspace(Workspace),
    Save(Save),
    Discard(Version),
    Features(Features),
    Diff(Diff),
    Conflicts(Diff),
    Resolve(Rebase),
    History(History),
    Commit(Commit),
    Publish(Publish),
    Rebase(Rebase),
    Restore(Restore),
}
