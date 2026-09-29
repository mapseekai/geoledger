use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Name {
    pub(super) name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Project {
    pub(super) project: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NamedProject {
    pub(super) project: String,
    pub(super) name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Member {
    pub(super) project: String,
    pub(super) subject: String,
    pub(super) role: String,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Page {
    #[serde(default)]
    pub(super) after: String,
    #[serde(default = "page_size")]
    pub(super) limit: i64,
}
pub(super) fn page_size() -> i64 {
    100
}
impl Page {
    pub(super) fn check(&self) -> Result<()> {
        if !(1..=1000).contains(&self.limit) || self.after.len() > 512 {
            Err(bad())
        } else {
            Ok(())
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectPage {
    pub(super) project: String,
    #[serde(default)]
    pub(super) after: String,
    #[serde(default = "page_size")]
    pub(super) limit: i64,
}
impl ProjectPage {
    pub(super) fn page(&self) -> Result<()> {
        Page {
            after: self.after.clone(),
            limit: self.limit,
        }
        .check()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Workspace {
    pub(super) project: String,
    pub(super) workspace: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Version {
    pub(super) project: String,
    pub(super) workspace: String,
    pub(super) expected_workspace_version: i64,
}
pub(super) fn membership(t: &mut Transaction<'_>, p: &str, s: &str, write: bool) -> Result<String> {
    id(p)?;
    let row = t.query_opt("SELECT role FROM _geoledger_center.project_members WHERE project=$1::text::uuid AND subject=$2 FOR SHARE", &[&p,&s])?.ok_or_else(missing)?;
    let role: String = row.get(0);
    if write && role == "viewer" {
        return Err(missing());
    }
    Ok(role)
}
pub(super) fn audit(
    t: &mut Transaction<'_>,
    p: &str,
    s: &str,
    action: &str,
    detail: Value,
) -> Result<()> {
    t.execute("INSERT INTO _geoledger_center.audit_events(project,subject,action,detail) VALUES($1::text::uuid,$2,$3,$4::text::jsonb)",&[&p,&s,&action,&detail.to_string()])?;
    Ok(())
}
pub(super) fn head(t: &mut Transaction<'_>, p: &str, lock: bool) -> Result<i64> {
    id(p)?;
    let sql = if lock {
        "SELECT head FROM _geoledger_center.projects WHERE id=$1::text::uuid FOR NO KEY UPDATE"
    } else {
        "SELECT head FROM _geoledger_center.projects WHERE id=$1::text::uuid"
    };
    Ok(t.query_opt(sql, &[&p])?.ok_or_else(missing)?.get(0))
}
pub(super) fn workspace(
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
pub(super) fn bump(t: &mut Transaction<'_>, p: &str, w: &str, status: &str) -> Result<i64> {
    Ok(t.query_one("UPDATE _geoledger_center.workspaces SET version=version+1,status=$3 WHERE project=$1::text::uuid AND id=$2::text::uuid RETURNING version",&[&p,&w,&status])?.get(0))
}
pub(super) fn create_project(t: &mut Transaction<'_>, s: &str, r: Name) -> Result<Value> {
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
pub(super) fn list_projects(t: &mut Transaction<'_>, s: &str, r: Page) -> Result<Value> {
    r.check()?;
    let rows=t.query("SELECT p.id::text,p.name,p.head FROM _geoledger_center.projects p JOIN _geoledger_center.project_members m ON m.project=p.id WHERE m.subject=$1 AND p.id::text>$2 ORDER BY p.id::text LIMIT $3",&[&s,&r.after,&r.limit])?;
    Ok(json!(rows.iter().map(|x|json!({"project":x.get::<_,String>(0),"name":x.get::<_,String>(1),"head":x.get::<_,i64>(2)})).collect::<Vec<_>>()))
}
pub(super) fn get_project(t: &mut Transaction<'_>, s: &str, r: Project) -> Result<Value> {
    let role = membership(t, &r.project, s, false)?;
    let row = t.query_one(
        "SELECT name,head FROM _geoledger_center.projects WHERE id=$1::text::uuid",
        &[&r.project],
    )?;
    Ok(
        json!({"project":r.project,"name":row.get::<_,String>(0),"head":row.get::<_,i64>(1),"role":role}),
    )
}
pub(super) fn set_member(t: &mut Transaction<'_>, s: &str, r: Member) -> Result<Value> {
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
pub(super) fn create_dataset(t: &mut Transaction<'_>, s: &str, r: NamedProject) -> Result<Value> {
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
pub(super) fn list_datasets(t: &mut Transaction<'_>, s: &str, r: ProjectPage) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    r.page()?;
    let rows=t.query("SELECT id::text,name FROM _geoledger_center.datasets WHERE project=$1::text::uuid AND id::text>$2 ORDER BY id::text LIMIT $3",&[&r.project,&r.after,&r.limit])?;
    Ok(json!(
        rows.iter()
            .map(|x| json!({"dataset":x.get::<_,String>(0),"name":x.get::<_,String>(1)}))
            .collect::<Vec<_>>()
    ))
}
pub(super) fn create_workspace(t: &mut Transaction<'_>, s: &str, r: Project) -> Result<Value> {
    membership(t, &r.project, s, true)?;
    let base = head(t, &r.project, false)?;
    new_workspace(t, s, &r.project, base)
}
pub(super) fn new_workspace(t: &mut Transaction<'_>, s: &str, p: &str, base: i64) -> Result<Value> {
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
pub(super) fn get_workspace(t: &mut Transaction<'_>, s: &str, r: Workspace) -> Result<Value> {
    let (b, v, status) = workspace(t, s, &r.project, &r.workspace, None)?;
    Ok(json!({"workspace":r.workspace,"base_revision":b,"version":v,"status":status}))
}
pub(super) fn list_workspaces(t: &mut Transaction<'_>, s: &str, r: ProjectPage) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    r.page()?;
    let rows=t.query("SELECT id::text,base_revision,version,status FROM _geoledger_center.workspaces WHERE project=$1::text::uuid AND owner=$2 AND id::text>$3 ORDER BY id::text LIMIT $4",&[&r.project,&s,&r.after,&r.limit])?;
    Ok(json!(rows.iter().map(|x|json!({"workspace":x.get::<_,String>(0),"base_revision":x.get::<_,i64>(1),"version":x.get::<_,i64>(2),"status":x.get::<_,String>(3)})).collect::<Vec<_>>()))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Edit {
    pub(super) dataset: String,
    pub(super) feature_id: String,
    #[serde(deserialize_with = "required_feature")]
    pub(super) feature: Option<Feature>,
}
pub(super) fn required_feature<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Feature>, D::Error> {
    Option::<Feature>::deserialize(d)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Save {
    pub(super) project: String,
    pub(super) workspace: String,
    pub(super) expected_workspace_version: i64,
    pub(super) edits: Vec<Edit>,
}
pub(super) fn put_delta(
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
pub(super) fn save(t: &mut Transaction<'_>, s: &str, r: Save) -> Result<Value> {
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
pub(super) fn discard(t: &mut Transaction<'_>, s: &str, r: Version) -> Result<Value> {
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
