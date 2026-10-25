use super::*;
use crate::repository::{
    AuditRepository, DatasetRepository, FeatureRepository, HistoryRepository, ProjectRepository,
    ResponseBudget, WorkspaceRepository,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Features {
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
pub(super) fn features(
    t: &mut (
             impl DatasetRepository
             + FeatureRepository
             + ProjectRepository
             + ResponseBudget
             + WorkspaceRepository
             + ?Sized
         ),
    s: &str,
    r: Features,
) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    dataset(t, &r.project, &r.dataset)?;
    let crs: String = t
        .dataset_exists(&r.project, &r.dataset)?
        .ok_or_else(missing)?
        .get(4usize)?;
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
        let (base, version, status) = workspace(t, s, &r.project, w, None)?;
        let revision = if status == "published" {
            t.workspace_publication_revision(&r.project, w)?
                .ok_or_else(|| Error::new(500, "missing workspace publication"))?
        } else {
            base
        };
        (revision, Some(version))
    } else {
        let h = head(t, &r.project, false)?;
        let v = r.revision.unwrap_or(h);
        if v < 0 || v > h {
            return Err(bad());
        }
        (v, None)
    };
    let definition =
        effective_projection(t, &r.project, &r.dataset, revision, r.workspace.as_deref())?;
    let bbox = r.bbox.unwrap_or([-180., -90., 180., 90.]);
    if bbox.iter().any(|x| !x.is_finite()) || bbox[0] > bbox[2] || bbox[1] > bbox[3] {
        return Err(bad());
    }
    // Remove ALL shadowed base rows before spatial filtering, including deletes and moves.
    let mut values = Vec::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    let mut query = crate::repository::FeatureQuery {
        project: r.project,
        dataset: r.dataset,
        revision,
        workspace: r.workspace,
        after: r.after,
        feature_id: r.feature_id,
        bbox: r.bbox,
        limit: r.limit,
    };
    while values.len() < r.limit as usize {
        query.limit = (r.limit - values.len() as i64).min(32);
        let rows = t.feature_page(&query)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            query.after = row.get(0usize)?;
            let mut properties: Value = codec::stored(&row.get::<_, String>(1usize)?)?;
            if let Some(properties) = properties.as_object_mut() {
                project_properties(properties, &definition);
            }
            let geometry: Value = row
                .get::<_, Option<String>>(2usize)?
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(Error::stored_json)?
                .unwrap_or(Value::Null);
            let item = json!({"type":"Feature","id":row.get::<_,String>(0usize)?,"properties":properties,"geometry":geometry});
            budget.include(&item)?;
            values.push(item);
        }
    }
    if query.feature_id.is_some() {
        let mut feature = values.pop().ok_or_else(missing)?;
        feature["crs"] = json!(crs);
        feature["revision"] = json!(revision);
        feature["workspace_version"] = json!(workspace_version);
        return Ok(feature);
    }
    // Probe the same visible snapshot without loading an extra feature's payload.
    let next = if values.len() == r.limit as usize && t.feature_exists(&query)? {
        Some(query.after)
    } else {
        None
    };
    Ok(
        json!({"type":"FeatureCollection","crs":crs,"features":values,"revision":revision,"workspace_version":workspace_version,"next_after":next}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Diff {
    pub(super) project: String,
    pub(super) workspace: String,
    #[serde(default)]
    pub(super) after: String,
    #[serde(default = "page_size")]
    pub(super) limit: i64,
}
pub(super) fn diff(
    t: &mut (impl DatasetRepository + ProjectRepository + ResponseBudget + WorkspaceRepository + ?Sized),
    s: &str,
    r: Diff,
) -> Result<Value> {
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    let mut result = Vec::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    let mut after = r.after;
    while result.len() < r.limit as usize {
        let limit = (r.limit - result.len() as i64).min(32);
        let (after_dataset, after_key) = cursor_parts(&after)?;
        let rows = t.diff_page(
            &r.project,
            &r.workspace,
            base,
            &after_dataset,
            &after_key,
            limit,
        )?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let dataset: String = row.get(0usize)?;
            let key: String = row.get(1usize)?;
            after = format!("{dataset}/{key}");
            let item = json!({"cursor":after,"dataset":dataset,"feature_id":key,"base":feature_row(&row,&key,4,5)?,"draft":feature_row(&row,&key,2,3)?});
            budget.include(&item)?;
            result.push(item);
        }
    }
    // Probe only keys in the same snapshot: an unreturned large feature must
    // not consume the current page's database/response byte budget.
    let next = if result.len() == r.limit as usize {
        let (dataset, key) = cursor_parts(&after)?;
        t.diff_has_after(&r.project, &r.workspace, &dataset, &key)?
            .then_some(after)
    } else {
        None
    };
    Ok(
        json!({"base_revision":base,"version":version,"changes":result,"next_after":next,"schema_changes":workspace_schema_diff(t,&r.project,&r.workspace,base)?}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct History {
    project: String,
    #[serde(default)]
    after: i64,
    #[serde(default = "page_size")]
    limit: i64,
}
pub(super) fn history(
    t: &mut (impl HistoryRepository + ProjectRepository + ?Sized),
    s: &str,
    r: History,
) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    if r.after < 0 || !(1..=1000).contains(&r.limit) {
        return Err(bad());
    }
    let rows = t.history_page(&r.project, r.after, r.limit)?;
    let next = match rows.last() {
        Some(row) if rows.len() == r.limit as usize => {
            let after = row.get::<_, i64>(0usize)?;
            t.history_has_after(&r.project, after)?.then_some(after)
        }
        _ => None,
    };
    let commits = rows.iter().map(|x|Ok(json!({"revision":x.get::<_,i64>(0usize)?,"subject":x.get::<_,String>(1usize)?,"message":x.get::<_,String>(2usize)?,"created_at":x.get::<_,String>(3usize)?,"source_workspace":x.get::<_,String>(4usize)?,"source_base_revision":x.get::<_,i64>(5usize)?}))).collect::<Result<Vec<_>>>()?;
    Ok(json!({"commits":commits,"next_after":next}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Commit {
    project: String,
    revision: i64,
    #[serde(default)]
    after: String,
    #[serde(default = "page_size")]
    limit: i64,
}
pub(super) fn commit_detail(
    t: &mut (impl DatasetRepository + HistoryRepository + ProjectRepository + ResponseBudget + ?Sized),
    s: &str,
    r: Commit,
) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    t.commit_exists(&r.project, r.revision)?
        .ok_or_else(missing)?;
    let mut out = Vec::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    let mut after = r.after;
    while out.len() < r.limit as usize {
        let limit = (r.limit - out.len() as i64).min(32);
        let (after_dataset, after_key) = cursor_parts(&after)?;
        let rows = t.commit_page(&r.project, r.revision, &after_dataset, &after_key, limit)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let d: String = row.get(0usize)?;
            let key: String = row.get(1usize)?;
            after = format!("{d}/{key}");
            let item = json!({"cursor":after,"dataset":d,"feature_id":key,"before":feature_row(&row,&key,2,3)?,"after":feature_row(&row,&key,4,5)?});
            budget.include(&item)?;
            out.push(item);
        }
    }
    let next = if out.len() == r.limit as usize {
        let (dataset, key) = cursor_parts(&after)?;
        t.commit_has_after(&r.project, r.revision, &dataset, &key)?
            .then_some(after)
    } else {
        None
    };
    Ok(
        json!({"revision":r.revision,"changes":out,"next_after":next,"schema_changes":commit_schema_diff(t,&r.project,r.revision)?}),
    )
}

fn feature_row(row: &Row, key: &str, properties: usize, geometry: usize) -> Result<Value> {
    let Some(properties) = row.get::<_, Option<String>>(properties)? else {
        return Ok(Value::Null);
    };
    let properties: Value = codec::stored(&properties)?;
    let geometry: Value = row
        .get::<_, Option<String>>(geometry)?
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(Error::stored_json)?
        .unwrap_or(Value::Null);
    Ok(json!({"type":"Feature","id":key,"properties":properties,"geometry":geometry}))
}

// The wire cursor remains dataset/feature_id; use native tuple comparisons so
// PostgreSQL can seek the existing primary-key indexes instead of sorting a
// concatenated expression for every page. Feature IDs may themselves contain '/'.
pub(super) fn cursor_parts(cursor: &str) -> Result<(String, String)> {
    if cursor.is_empty() {
        return Ok((Uuid::nil().to_string(), String::new()));
    }
    let (dataset, key) = cursor.split_once('/').ok_or_else(bad)?;
    text(key, 256)?;
    let dataset = Uuid::parse_str(dataset).map_err(|_| bad())?;
    Ok((dataset.to_string(), key.to_owned()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Audit {
    project: String,
    #[serde(default)]
    after: i64,
    #[serde(default = "page_size")]
    limit: i64,
}
pub(super) fn audit_events(
    t: &mut (impl AuditRepository + ProjectRepository + ResponseBudget + ?Sized),
    subject: &str,
    r: Audit,
) -> Result<Value> {
    if membership(t, &r.project, subject, false)? != "owner" {
        return Err(missing());
    }
    if r.after < 0 || !(1..=1000).contains(&r.limit) {
        return Err(bad());
    }
    let rows = t.audit_page(&r.project, r.after, r.limit)?;
    let next = match rows.last() {
        Some(row) if rows.len() == r.limit as usize => {
            let after = row.get::<_, i64>(0usize)?;
            t.audit_has_after(&r.project, after)?.then_some(after)
        }
        _ => None,
    };
    let mut events = Vec::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    for row in rows {
        let detail: Value = codec::stored(&row.get::<_, String>(3usize)?)?;
        let event = json!({"id":row.get::<_,i64>(0usize)?,"subject":row.get::<_,String>(1usize)?,"action":row.get::<_,String>(2usize)?,"detail":detail,"created_at":row.get::<_,String>(4usize)?});
        budget.include(&event)?;
        events.push(event);
    }
    Ok(json!({"events":events,"next_after":next}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SummaryCommit {
    project: String,
    revision: i64,
}
pub(super) fn workspace_summary(
    t: &mut (impl DatasetRepository + ProjectRepository + WorkspaceRepository + ?Sized),
    s: &str,
    r: Workspace,
) -> Result<Value> {
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    let mut result = change_summary(t.workspace_summary(&r.project, &r.workspace, base)?)?;
    result["version"] = json!(version);
    result["schema_changes"] = workspace_schema_diff(t, &r.project, &r.workspace, base)?;
    Ok(result)
}
pub(super) fn commit_summary(
    t: &mut (impl DatasetRepository + HistoryRepository + ProjectRepository + ?Sized),
    s: &str,
    r: SummaryCommit,
) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    t.commit_exists(&r.project, r.revision)?
        .ok_or_else(missing)?;
    let mut result = change_summary(t.commit_summary(&r.project, r.revision)?)?;
    result["revision"] = json!(r.revision);
    result["schema_changes"] = commit_schema_diff(t, &r.project, r.revision)?;
    Ok(result)
}
fn change_summary(rows: Vec<Row>) -> Result<Value> {
    let mut total = [0_i64; 3];
    let mut datasets = Vec::new();
    for row in rows {
        let added: i64 = row.get(2usize)?;
        let deleted: i64 = row.get(3usize)?;
        let modified: i64 = row.get(4usize)?;
        total[0] += added;
        total[1] += deleted;
        total[2] += modified;
        datasets.push(json!({"id":row.get::<_,String>(0usize)?,"name":row.get::<_,String>(1usize)?,"added":added,"deleted":deleted,"modified":modified}));
    }
    Ok(
        json!({"total":{"added":total[0],"deleted":total[1],"modified":total[2]},"datasets":datasets}),
    )
}
