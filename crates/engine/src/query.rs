use super::*;

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
pub(super) fn features(t: &mut Transaction, s: &str, r: Features) -> Result<Value> {
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
    let mut values = Vec::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    let mut after = r.after.clone();
    while values.len() < r.limit as usize {
        let batch_limit = (r.limit - values.len() as i64).min(32);
        let rows = t.feature_page(&crate::repository::FeatureQuery {
            project: r.project.clone(),
            dataset: r.dataset.clone(),
            revision,
            workspace: r.workspace.clone(),
            after: after.clone(),
            feature_id: r.feature_id.clone(),
            bbox: r.bbox,
            limit: batch_limit,
        })?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row.get(0usize)?;
            let properties: Value = codec::stored(&row.get::<_, String>(1usize)?)?;
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
pub(super) fn diff(t: &mut Transaction, s: &str, r: Diff) -> Result<Value> {
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
    Ok(json!({"base_revision":base,"version":version,"changes":result}))
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
pub(super) fn history(t: &mut Transaction, s: &str, r: History) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    if r.after < 0 || !(1..=1000).contains(&r.limit) {
        return Err(bad());
    }
    let rows = t.history_page(&r.project, r.after, r.limit)?;
    Ok(json!(rows.iter().map(|x|Ok(json!({"revision":x.get::<_,i64>(0usize)?,"subject":x.get::<_,String>(1usize)?,"message":x.get::<_,String>(2usize)?,"created_at":x.get::<_,String>(3usize)?,"source_workspace":x.get::<_,String>(4usize)?,"source_base_revision":x.get::<_,i64>(5usize)?}))).collect::<Result<Vec<_>>>()?))
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
pub(super) fn commit_detail(t: &mut Transaction, s: &str, r: Commit) -> Result<Value> {
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
    Ok(json!({"revision":r.revision,"changes":out}))
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
pub(super) fn audit_events(t: &mut Transaction, subject: &str, r: Audit) -> Result<Value> {
    if membership(t, &r.project, subject, false)? != "owner" {
        return Err(missing());
    }
    if r.after < 0 || !(1..=1000).contains(&r.limit) {
        return Err(bad());
    }
    let rows = t.audit_page(&r.project, r.after, r.limit)?;
    let mut events = Vec::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    for row in rows {
        let detail: Value = codec::stored(&row.get::<_, String>(3usize)?)?;
        let event = json!({"id":row.get::<_,i64>(0usize)?,"subject":row.get::<_,String>(1usize)?,"action":row.get::<_,String>(2usize)?,"detail":detail,"created_at":row.get::<_,String>(4usize)?});
        budget.include(&event)?;
        events.push(event);
    }
    let next = events.last().map(|e| e["id"].clone());
    Ok(json!({"events":events,"next_after":next}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SummaryCommit {
    project: String,
    revision: i64,
}
pub(super) fn workspace_summary(t: &mut Transaction, s: &str, r: Workspace) -> Result<Value> {
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    let mut result = change_summary(t.workspace_summary(&r.project, &r.workspace, base)?)?;
    result["version"] = json!(version);
    Ok(result)
}
pub(super) fn commit_summary(t: &mut Transaction, s: &str, r: SummaryCommit) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    t.commit_exists(&r.project, r.revision)?
        .ok_or_else(missing)?;
    let mut result = change_summary(t.commit_summary(&r.project, r.revision)?)?;
    result["revision"] = json!(r.revision);
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
