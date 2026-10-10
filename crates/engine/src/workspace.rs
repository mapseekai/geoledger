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
/// Data access for an active member. Writes need editor/owner and an active project.
pub(super) fn membership(t: &mut Transaction, p: &str, s: &str, write: bool) -> Result<String> {
    id(p)?;
    let row = t.member_role(p, s)?.ok_or_else(missing)?;
    let role: String = row.get(0usize)?;
    let state: String = row.get(1usize)?;
    if write && role == "viewer" {
        return Err(missing());
    }
    if write && state == "archived" {
        return Err(Error::new(
            409,
            "project is archived; unarchive it before making changes",
        ));
    }
    Ok(role)
}
/// Project administration (members, archive, delete): active owners and
/// platform administrators. Returns (role, state); administrators without
/// membership report role "admin".
pub(super) fn manage(
    t: &mut Transaction,
    p: &str,
    s: &str,
    policy: &Policy,
    write: bool,
) -> Result<(String, String)> {
    id(p)?;
    if let Some(row) = t.member_role(p, s)? {
        let role: String = row.get(0usize)?;
        if role == "owner" {
            return Ok((role, row.get(1usize)?));
        }
    }
    if !policy.admins.contains(s) {
        return Err(missing());
    }
    let row = t.live_project(p, false)?.ok_or_else(missing)?;
    if write {
        t.ensure_identity(p, s)?;
    }
    let role = match t.member_role(p, s)? {
        Some(member) => member.get(0usize)?,
        None => "admin".to_owned(),
    };
    Ok((role, row.get(2usize)?))
}
/// Read access to project metadata: any active member or an administrator.
fn visible(t: &mut Transaction, p: &str, s: &str, policy: &Policy) -> Result<String> {
    id(p)?;
    if let Some(row) = t.member_role(p, s)? {
        return row.get(0usize);
    }
    if policy.admins.contains(s) && t.live_project(p, false)?.is_some() {
        return Ok("admin".into());
    }
    Err(missing())
}
pub(super) fn audit(
    t: &mut Transaction,
    p: &str,
    s: &str,
    action: &str,
    detail: Value,
) -> Result<()> {
    t.append_audit(p, s, action, &detail.to_string())?;
    Ok(())
}
pub(super) fn head(t: &mut Transaction, p: &str, lock: bool) -> Result<i64> {
    id(p)?;
    t.project_head(p, lock)?.ok_or_else(missing)?.get(0usize)
}
pub(super) fn workspace(
    t: &mut Transaction,
    s: &str,
    p: &str,
    w: &str,
    version: Option<i64>,
) -> Result<(i64, i64, String)> {
    if version.is_some() {
        head(t, p, true)?;
    }
    membership(t, p, s, version.is_some())?;
    id(w)?;
    let r = t
        .workspace_state(p, w, s, version.is_some())?
        .ok_or_else(missing)?;
    let v: i64 = r.get(1usize)?;
    let status: String = r.get(2usize)?;
    if version.is_some_and(|x| x != v || status != "open") {
        return Err(stale());
    }
    Ok((r.get(0usize)?, v, status))
}
pub(super) fn bump(t: &mut Transaction, p: &str, w: &str, status: &str) -> Result<i64> {
    t.advance_workspace(p, w, status)?.get(0usize)
}
pub(super) fn create_project(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: Name,
) -> Result<Value> {
    text(&r.name, 256)?;
    let admin = policy.admins.contains(s);
    if policy.admin_only_project_creation && !admin {
        return Err(Error::new(
            403,
            "project creation is restricted to administrators",
        ));
    }
    if let Some(max) = policy.max_owned_projects.filter(|_| !admin)
        && t.owned_projects(s)?.get::<_, i64>(0usize)? >= i64::from(max)
    {
        return Err(Error::new(403, "project quota exceeded"));
    }
    let p = Uuid::new_v4().to_string();
    t.insert_project(&p, &r.name)?;
    t.insert_owner(&p, s)?;
    audit(t, &p, s, "create_project", json!({}))?;
    Ok(json!({"project":p,"name":r.name,"head":0,"role":"owner","state":"active"}))
}
pub(super) fn list_projects(t: &mut Transaction, s: &str, r: Page) -> Result<Value> {
    r.check()?;
    let rows = t.list_projects(s, &r.after, r.limit)?;
    Ok(json!(rows.iter().map(|x|Ok(json!({"project":x.get::<_,String>(0usize)?,"name":x.get::<_,String>(1usize)?,"head":x.get::<_,i64>(2usize)?,"state":x.get::<_,String>(3usize)?,"role":x.get::<_,String>(4usize)?}))).collect::<Result<Vec<_>>>()?))
}
fn project_json(t: &mut Transaction, p: &str, role: &str) -> Result<Value> {
    let row = t.project_info(p)?;
    Ok(
        json!({"project":p,"name":row.get::<_,String>(0usize)?,"head":row.get::<_,i64>(1usize)?,"role":role,"state":row.get::<_,String>(2usize)?}),
    )
}
pub(super) fn get_project(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: Project,
) -> Result<Value> {
    let role = visible(t, &r.project, s, policy)?;
    project_json(t, &r.project, &role)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemberRef {
    pub(super) project: String,
    pub(super) subject: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Archive {
    pub(super) project: String,
    pub(super) archived: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteProject {
    pub(super) project: String,
    pub(super) confirm_name: String,
}
pub(super) fn list_members(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: ProjectPage,
) -> Result<Value> {
    visible(t, &r.project, s, policy)?;
    r.page()?;
    let rows = t.list_members(&r.project, &r.after, r.limit)?;
    Ok(json!(
        rows.iter()
            .map(|x| Ok(
                json!({"subject":x.get::<_,String>(0usize)?,"role":x.get::<_,String>(1usize)?})
            ))
            .collect::<Result<Vec<_>>>()?
    ))
}
pub(super) fn remove_member(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: MemberRef,
) -> Result<Value> {
    // Same lock order as set_member and publication.
    head(t, &r.project, true)?;
    text(&r.subject, 128)?;
    if r.subject == s {
        // Any active member may leave a project.
        membership(t, &r.project, s, false)?;
    } else {
        manage(t, &r.project, s, policy, true)?;
    }
    t.member_role(&r.project, &r.subject)?.ok_or_else(missing)?;
    let owners = t.owner_summary(&r.project, &r.subject)?;
    if owners.get::<_, i64>(1usize)? != 0 && owners.get::<_, i64>(0usize)? == 1 {
        return Err(Error::new(409, "project requires an owner"));
    }
    t.remove_member(&r.project, &r.subject)?;
    audit(
        t,
        &r.project,
        s,
        "remove_member",
        json!({"subject":r.subject}),
    )?;
    Ok(json!({"ok":true}))
}
pub(super) fn archive_project(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: Archive,
) -> Result<Value> {
    id(&r.project)?;
    t.live_project(&r.project, true)?.ok_or_else(missing)?;
    let (role, state) = manage(t, &r.project, s, policy, true)?;
    let next = if r.archived { "archived" } else { "active" };
    if state != next {
        t.set_project_state(&r.project, next)?;
        audit(
            t,
            &r.project,
            s,
            if r.archived {
                "archive_project"
            } else {
                "unarchive_project"
            },
            json!({}),
        )?;
    }
    project_json(t, &r.project, &role)
}
pub(super) fn delete_project(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: DeleteProject,
) -> Result<Value> {
    id(&r.project)?;
    let row = t.live_project(&r.project, true)?.ok_or_else(missing)?;
    manage(t, &r.project, s, policy, true)?;
    if row.get::<_, String>(0usize)? != r.confirm_name {
        return Err(Error::new(400, "confirm_name must equal the project name"));
    }
    t.purge_data(&r.project, None)?;
    t.set_project_state(&r.project, "deleted")?;
    audit(t, &r.project, s, "delete_project", json!({}))?;
    Ok(json!({"ok":true}))
}
pub(super) fn set_member(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: Member,
) -> Result<Value> {
    // Serialize membership management with publication; consistent lock order.
    head(t, &r.project, true)?;
    manage(t, &r.project, s, policy, true)?;
    text(&r.subject, 128)?;
    if !["owner", "editor", "viewer"].contains(&r.role.as_str()) {
        return Err(bad());
    }
    let owners = t.owner_summary(&r.project, &r.subject)?;
    if r.role != "owner"
        && (owners.get::<_, i64>(1usize)? != 0)
        && owners.get::<_, i64>(0usize)? == 1
    {
        return Err(Error::new(409, "project requires an owner"));
    }
    t.set_member(&r.project, &r.subject, &r.role)?;
    audit(
        t,
        &r.project,
        s,
        "set_member",
        json!({"subject":r.subject,"role":r.role}),
    )?;
    Ok(json!({"ok":true}))
}
pub(super) fn create_dataset(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: CreateDataset,
) -> Result<Value> {
    let current = head(t, &r.project, true)?;
    membership(t, &r.project, s, true)?;
    text(&r.name, 256)?;
    let d = Uuid::new_v4().to_string();
    let (family, requested_dimension) = if let Some(source) = &r.postgis_table {
        if !policy.admins.contains(s) {
            return Err(Error::new(
                403,
                "attaching a business table requires a platform administrator",
            ));
        }
        let row = t.inspect_postgis_table(source)?;
        (row.get::<_, String>(0usize)?, row.get::<_, i32>(1usize)?)
    } else {
        (r.geometry_type.clone(), r.coordinate_dimension)
    };
    if !["point", "line", "polygon"].contains(&family.as_str()) {
        return Err(Error::new(
            400,
            "geometry_type must be point, line or polygon",
        ));
    }
    let dimension = if requested_dimension == 0 {
        2
    } else {
        requested_dimension
    };
    if ![2, 3].contains(&dimension) {
        return Err(Error::new(400, "coordinate_dimension must be 2 or 3"));
    }
    t.insert_dataset(&r.project, &d, &r.name, &family, dimension)?;
    if let Some(source) = &r.postgis_table {
        let w = Uuid::new_v4().to_string();
        t.insert_workspace(&r.project, &w, s, current)?;
        t.begin_merge()?;
        let mut after = String::new();
        loop {
            let rows = t.postgis_table_page(source, &after)?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let key: String = row.get(0usize)?;
                let properties: String = row.get(1usize)?;
                let geometry: Option<String> = row.get(2usize)?;
                let feature = Feature {
                    kind: "Feature".into(),
                    id: key.clone(),
                    properties: codec::stored(&properties)?,
                    geometry: geometry
                        .map(|v| codec::stored(&v))
                        .transpose()?
                        .unwrap_or(Value::Null),
                };
                let value = normalize(feature, &key)?;
                validate_dataset_geometry(t, &r.project, &d, &value)?;
                t.stage_merge(
                    &d,
                    &key,
                    &None,
                    &Some(serde_json::to_string(&value).map_err(Error::stored_json)?),
                )?;
                after = key;
            }
        }
        let revision = current.checked_add(1).ok_or_else(bad)?;
        t.append_commit(&r.project, revision, &w, s, "Attach PostGIS table")?;
        t.append_changes(&r.project, revision)?;
        t.append_history(&r.project, revision)?;
        t.advance_head(&r.project, revision)?;
        t.advance_workspace(&r.project, &w, "published")?;
        t.bind_postgis_table(&r.project, &d, source)?;
    }
    audit(t, &r.project, s, "create_dataset", json!({"dataset":d}))?;
    Ok(
        json!({"dataset":d,"name":r.name,"geometry_type":family,"coordinate_dimension":dimension,"postgis_table":r.postgis_table}),
    )
}
pub(super) fn list_datasets(t: &mut Transaction, s: &str, r: ProjectPage) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    r.page()?;
    let rows = t.list_datasets(&r.project, &r.after, r.limit)?;
    Ok(json!(
        rows.iter()
            .map(|x| Ok(
                json!({"dataset":x.get::<_,String>(0usize)?,"name":x.get::<_,String>(1usize)?,"geometry_type":x.get::<_,String>(2usize)?,"coordinate_dimension":x.get::<_,i32>(3usize)?,"postgis_table":source_table(x.get(4usize)?)?})
            ))
            .collect::<Result<Vec<_>>>()?
    ))
}
pub(super) fn create_workspace(t: &mut Transaction, s: &str, r: Project) -> Result<Value> {
    membership(t, &r.project, s, true)?;
    let base = head(t, &r.project, true)?;
    new_workspace(t, s, &r.project, base)
}
pub(super) fn new_workspace(t: &mut Transaction, s: &str, p: &str, base: i64) -> Result<Value> {
    let w = Uuid::new_v4().to_string();
    t.insert_workspace(p, &w, s, base)?;
    audit(
        t,
        p,
        s,
        "create_workspace",
        json!({"workspace":w,"base_revision":base}),
    )?;
    Ok(json!({"workspace":w,"base_revision":base,"version":0,"status":"open"}))
}
pub(super) fn get_workspace(t: &mut Transaction, s: &str, r: Workspace) -> Result<Value> {
    let (b, v, status) = workspace(t, s, &r.project, &r.workspace, None)?;
    Ok(json!({"workspace":r.workspace,"base_revision":b,"version":v,"status":status}))
}
pub(super) fn list_workspaces(t: &mut Transaction, s: &str, r: ProjectPage) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    r.page()?;
    let rows = t.list_workspaces(&r.project, s, &r.after, r.limit)?;
    Ok(json!(rows.iter().map(|x|Ok(json!({"workspace":x.get::<_,String>(0usize)?,"base_revision":x.get::<_,i64>(1usize)?,"version":x.get::<_,i64>(2usize)?,"status":x.get::<_,String>(3usize)?}))).collect::<Result<Vec<_>>>()?))
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
    t: &mut Transaction,
    p: &str,
    w: &str,
    d: &str,
    key: &str,
    v: Option<&Stored>,
) -> Result<()> {
    if let Some(v) = v {
        validate_dataset_geometry(t, p, d, v)?;
    }
    let props = v.map(|x| json!(x.properties).to_string());
    let g = v.and_then(|x| x.geometry.as_deref());
    t.put_delta(p, w, d, key, &props, &g)?;
    Ok(())
}
pub(super) fn save(t: &mut Transaction, s: &str, r: Save) -> Result<Value> {
    head(t, &r.project, true)?;
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
    t.invalidate_resolutions(&r.project, &r.workspace)?;
    let mut seen = BTreeSet::new();
    let mut warnings = Vec::new();
    for e in r.edits {
        dataset(t, &r.project, &e.dataset)?;
        text(&e.feature_id, 256)?;
        if !seen.insert((e.dataset.clone(), e.feature_id.clone())) {
            return Err(bad());
        }
        let value = e
            .feature
            .map(|f| {
                let value = normalize(f.clone(), &e.feature_id)?;
                if let Some(warning) = geometry::topology_warning(&f.geometry)? {
                    warnings.push(format!(
                        "dataset {}, feature {}: {}",
                        e.dataset, e.feature_id, warning
                    ));
                }
                Ok::<_, Error>(value)
            })
            .transpose()?;
        let was_resolved: bool = t
            .has_resolution(&r.project, &r.workspace, &e.dataset, &e.feature_id)?
            .get(0usize)?;
        if !was_resolved && value == at_revision(t, &r.project, &e.dataset, &e.feature_id, base)? {
            t.remove_delta(&r.project, &r.workspace, &e.dataset, &e.feature_id)?;
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
    let count: i64 = t.count_deltas(&r.project, &r.workspace)?.get(0usize)?;
    let version = bump(t, &r.project, &r.workspace, "open")?;
    audit(
        t,
        &r.project,
        s,
        "save",
        json!({"workspace":r.workspace,"version":version,"topology_warnings":warnings}),
    )?;
    Ok(json!({"version":version,"changes":count,"warnings":warnings}))
}
pub(super) fn discard(t: &mut Transaction, s: &str, r: Version) -> Result<Value> {
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
pub(super) struct CreateDataset {
    project: String,
    name: String,
    #[serde(default)]
    geometry_type: String,
    #[serde(default)]
    coordinate_dimension: i32,
    postgis_table: Option<PostgisTable>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RenameDataset {
    project: String,
    dataset: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteDataset {
    project: String,
    dataset: String,
    confirm_name: String,
}
pub(super) fn rename_project(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: NamedProject,
) -> Result<Value> {
    head(t, &r.project, true)?;
    let (role, _) = manage(t, &r.project, s, policy, true)?;
    text(&r.name, 256)?;
    t.rename_project(&r.project, &r.name)?;
    audit(t, &r.project, s, "rename_project", json!({"name":r.name}))?;
    project_json(t, &r.project, &role)
}
pub(super) fn rename_dataset(t: &mut Transaction, s: &str, r: RenameDataset) -> Result<Value> {
    head(t, &r.project, true)?;
    membership(t, &r.project, s, true)?;
    dataset(t, &r.project, &r.dataset)?;
    text(&r.name, 256)?;
    let row = t
        .dataset_exists(&r.project, &r.dataset)?
        .ok_or_else(missing)?;
    t.rename_dataset(&r.project, &r.dataset, &r.name)?;
    audit(
        t,
        &r.project,
        s,
        "rename_dataset",
        json!({"dataset":r.dataset,"name":r.name}),
    )?;
    Ok(
        json!({"dataset":r.dataset,"name":r.name,"geometry_type":row.get::<_,String>(0usize)?,"coordinate_dimension":row.get::<_,i32>(2usize)?,"postgis_table":source_table(row.get(3usize)?)?}),
    )
}
pub(super) fn delete_dataset(
    t: &mut Transaction,
    s: &str,
    policy: &Policy,
    r: DeleteDataset,
) -> Result<Value> {
    head(t, &r.project, true)?;
    manage(t, &r.project, s, policy, true)?;
    id(&r.dataset)?;
    let row = t
        .dataset_exists(&r.project, &r.dataset)?
        .ok_or_else(missing)?;
    if row.get::<_, String>(1usize)? != r.confirm_name {
        return Err(Error::new(400, "confirm_name must equal the dataset name"));
    }
    t.purge_data(&r.project, Some(&r.dataset))?;
    audit(
        t,
        &r.project,
        s,
        "delete_dataset",
        json!({"dataset":r.dataset,"name":r.confirm_name}),
    )?;
    Ok(json!({"ok":true}))
}

fn source_table(raw: Option<String>) -> Result<Value> {
    Ok(raw
        .map(|s| codec::stored::<Value>(&s).map(|v| v["table"].clone()))
        .transpose()?
        .unwrap_or(Value::Null))
}
