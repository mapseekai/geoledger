use super::*;

struct MergedRow {
    d: Delta,
    b: Option<Stored>,
    o: Option<Stored>,
    stale_resolution: bool,
    merged: std::result::Result<Option<Stored>, Vec<String>>,
}
impl MergedRow {
    fn from_row(row: &Row, current: i64) -> Result<Self> {
        let d = Delta {
            dataset: row.get(0usize)?,
            key: row.get(1usize)?,
            value: stored_row(row, 2, 3)?,
            resolved_head: row.get(4usize)?,
            resolution_stale: row.get(5usize)?,
        };
        let b = stored_row(row, 6, 7)?;
        let o = stored_row(row, 8, 9)?;
        let stale_resolution = d
            .resolved_head
            .is_some_and(|h| h != current || d.resolution_stale);
        let merged = if stale_resolution {
            Err(Vec::new())
        } else if d.resolved_head.is_some() {
            Ok(d.value.clone())
        } else {
            match merge_record(
                b.as_ref().map(|v| v.record(&d.key)).as_ref(),
                o.as_ref().map(|v| v.record(&d.key)).as_ref(),
                d.value.as_ref().map(|v| v.record(&d.key)).as_ref(),
            ) {
                Ok(v) => Ok(v.map(Stored::from_record).transpose()?),
                Err(fields) => Err(fields),
            }
        };
        Ok(Self {
            d,
            b,
            o,
            stale_resolution,
            merged,
        })
    }
    fn conflict(self) -> Result<Value> {
        let fields = self
            .merged
            .err()
            .ok_or_else(|| Error::new(500, "conflict index does not match its version"))?;
        conflict_item(
            &self.d,
            self.b.as_ref(),
            self.o.as_ref(),
            fields,
            self.stale_resolution,
        )
    }
}
fn conflict_item(
    d: &Delta,
    b: Option<&Stored>,
    o: Option<&Stored>,
    fields: Vec<String>,
    stale_resolution: bool,
) -> Result<Value> {
    let mut item = json!({"cursor":format!("{}/{}",d.dataset,d.key),"dataset":d.dataset,"feature_id":d.key,"fields":fields,"base":geojson(&d.key,b)?,"current":geojson(&d.key,o)?,"draft":geojson(&d.key,d.value.as_ref())?});
    if stale_resolution {
        item["reason"] = json!("stale_resolution");
        item["resolved_against_revision"] = json!(d.resolved_head);
    }
    Ok(item)
}

pub(super) struct MergePlan {
    change_count: usize,
    conflict_count: usize,
    conflicts: Vec<Value>,
    conflict_keys: BTreeSet<(String, String)>,
    truncated: bool,
}
// Only one batch of expanded features is retained. Successful candidates are
// staged in the transaction, not accumulated in a Rust Vec across the workspace.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MergeMode {
    Inspect,
    Stage,
    CacheKeys,
}
pub(super) fn merge_plan(
    t: &mut Transaction,
    p: &str,
    w: &str,
    base: i64,
    current: i64,
    page: Option<(&str, usize)>,
    mode: MergeMode,
) -> Result<MergePlan> {
    let stage = mode == MergeMode::Stage;
    let cache_keys = mode == MergeMode::CacheKeys;
    if stage {
        t.begin_merge()?;
    }
    let mut plan = MergePlan {
        change_count: 0,
        conflict_count: 0,
        conflicts: Vec::new(),
        conflict_keys: BTreeSet::new(),
        truncated: false,
    };
    let mut after = String::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    // Restrict the workspace prefix before expanding joined historical JSON.
    loop {
        let (after_dataset, after_key) = cursor_parts(&after)?;
        let rows = t.merge_page(p, w, base, current, &after_dataset, &after_key)?;
        if rows.is_empty() {
            break;
        }
        let mut cache_batch = Vec::new();
        for row in rows {
            let MergedRow {
                d,
                b,
                o,
                stale_resolution,
                merged,
            } = MergedRow::from_row(&row, current)?;
            after = format!("{}/{}", d.dataset, d.key);
            match merged {
                Ok(value) => {
                    if let Some(v) = &value {
                        validate_candidate(v)?;
                        validate_dataset_geometry(t, p, &d.dataset, v)?;
                    }
                    if value != o {
                        plan.change_count += 1;
                        if stage {
                            t.stage_merge(
                                &d.dataset,
                                &d.key,
                                &o.as_ref()
                                    .map(serde_json::to_string)
                                    .transpose()
                                    .map_err(Error::stored_json)?,
                                &value
                                    .as_ref()
                                    .map(serde_json::to_string)
                                    .transpose()
                                    .map_err(Error::stored_json)?,
                            )?;
                        }
                    }
                }
                Err(fields) => {
                    plan.conflict_count += 1;
                    if cache_keys {
                        cache_batch.push((d.dataset.clone(), d.key.clone()));
                    } else {
                        plan.conflict_keys
                            .insert((d.dataset.clone(), d.key.clone()));
                    }
                    if let Some((cursor, limit)) = page
                        && after.as_str() > cursor
                    {
                        if plan.conflicts.len() >= limit || plan.truncated {
                            plan.truncated = true;
                            continue;
                        }
                        let item =
                            conflict_item(&d, b.as_ref(), o.as_ref(), fields, stale_resolution)?;
                        budget.include(&item)?;
                        plan.conflicts.push(item);
                    }
                }
            }
        }
        if cache_keys {
            t.cache_conflict_keys(p, w, &cache_batch)?;
        }
    }
    Ok(plan)
}
pub(super) fn conflicts(head: i64, version: i64, plan: MergePlan) -> Error {
    let mut error = Error::new(409, "merge conflicts");
    let next = plan.conflicts.last().map(|v| v["cursor"].clone());
    error.body["head"] = json!(head);
    error.body["version"] = json!(version);
    error.body["total"] = json!(plan.conflict_keys.len());
    error.body["next_after"] = next.unwrap_or(Value::Null);
    error.body["truncated"] = json!(plan.truncated);
    error.body["conflicts"] = Value::Array(plan.conflicts);
    error
}
pub(super) fn list_conflicts(t: &mut Transaction, s: &str, r: Diff) -> Result<Value> {
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    let (mut after_dataset, mut after_key) = cursor_parts(&r.after)?;
    // Serialize cache construction with mutations; every subsequent page still authorizes.
    let current = head(t, &r.project, true)?;
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    let total = match t.cached_conflict_count(&r.project, &r.workspace, base, version, current)? {
        Some(total) => total,
        None => {
            t.reset_conflict_cache(&r.project, &r.workspace, base, version, current)?;
            let plan = merge_plan(
                t,
                &r.project,
                &r.workspace,
                base,
                current,
                None,
                MergeMode::CacheKeys,
            )?;
            let total = i64::try_from(plan.conflict_count).map_err(|_| bad())?;
            t.complete_conflict_cache(&r.project, &r.workspace, total)?;
            total
        }
    };
    let mut values = Vec::new();
    let mut budget = codec::Budget::new(t.response_limit(), t.response_reservation());
    let mut truncated = false;
    'pages: loop {
        let limit = ((r.limit as usize).saturating_sub(values.len()) + 1).min(32) as i64;
        let rows = t.cached_conflict_page(&repository::ConflictQuery {
            project: &r.project,
            workspace: &r.workspace,
            base,
            head: current,
            after_dataset: &after_dataset,
            after_key: &after_key,
            limit,
        })?;
        let full = rows.len() == limit as usize;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            if values.len() == r.limit as usize {
                truncated = true;
                break 'pages;
            }
            after_dataset = row.get(0usize)?;
            after_key = row.get(1usize)?;
            let item = MergedRow::from_row(&row, current)?.conflict()?;
            budget.include(&item)?;
            values.push(item);
        }
        if !full {
            break;
        }
    }
    let next = values.last().map(|v| v["cursor"].clone());
    Ok(
        json!({"head":current,"version":version,"conflicts":values,"total":total,"next_after":next,"truncated":truncated}),
    )
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Publish {
    project: String,
    workspace: String,
    expected_workspace_version: i64,
    request_id: Uuid,
    message: String,
}
pub(super) fn publish(t: &mut Transaction, s: &str, r: Publish) -> Result<Value> {
    text(&r.message, 2048)?;
    let current = head(t, &r.project, true)?;
    // A receipt replay is a read, including after archive or role downgrade.
    // Keep membership and workspace ownership checks ahead of receipt access.
    workspace(t, s, &r.project, &r.workspace, None)?;
    let request_id = r.request_id.to_string();
    let payload = serde_json::to_value(&r).map_err(|_| bad())?;
    if let Some(row) = t.publication_receipt(&r.project, s, &request_id)? {
        let previous: Value = codec::stored(&row.get::<_, String>(0usize)?)?;
        if previous != payload {
            return Err(Error::new(409, "request_id payload mismatch"));
        }
        return codec::stored(&row.get::<_, String>(1usize)?);
    }
    let (base, version, _) = workspace(
        t,
        s,
        &r.project,
        &r.workspace,
        Some(r.expected_workspace_version),
    )?;
    let plan = merge_plan(
        t,
        &r.project,
        &r.workspace,
        base,
        current,
        Some(("", 100)),
        MergeMode::Stage,
    )?;
    if !plan.conflict_keys.is_empty() {
        return Err(conflicts(current, version, plan));
    }
    let revision = current.checked_add(1).ok_or_else(bad)?;
    t.append_commit(&r.project, revision, &r.workspace, s, &r.message)?;
    // Set-based writes use the already validated, three-way merged candidates.
    t.append_changes(&r.project, revision)?;
    t.close_history(&r.project, revision)?;
    t.append_history(&r.project, revision)?;
    t.advance_head(&r.project, revision)?;
    let v = bump(t, &r.project, &r.workspace, "published")?;
    let result = json!({"revision":revision,"workspace":r.workspace,"version":v,"status":"published","changes":plan.change_count});
    t.save_receipt(
        &r.project,
        s,
        &request_id,
        &payload.to_string(),
        &result.to_string(),
    )?;
    audit(t, &r.project, s, "publish", result.clone())?;
    Ok(result)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rebase {
    project: String,
    workspace: String,
    expected_workspace_version: i64,
    expected_head: i64,
    resolutions: Vec<Edit>,
}
pub(super) fn resolve(t: &mut Transaction, s: &str, r: Rebase) -> Result<Value> {
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
    let plan = merge_plan(
        t,
        &r.project,
        &r.workspace,
        base,
        current,
        None,
        MergeMode::Inspect,
    )?;
    let mut remaining = plan.conflict_keys;
    for e in r.resolutions {
        if !remaining.remove(&(e.dataset.clone(), e.feature_id.clone())) {
            return Err(Error::new(
                409,
                "resolution must identify a current conflict exactly once",
            ));
        }
        let value = e.feature.map(|f| normalize(f, &e.feature_id)).transpose()?;
        if let Some(v) = &value {
            validate_dataset_geometry(t, &r.project, &e.dataset, v)?;
        }
        put_delta(
            t,
            &r.project,
            &r.workspace,
            &e.dataset,
            &e.feature_id,
            value.as_ref(),
        )?;
        t.mark_resolved(&r.project, &r.workspace, &e.dataset, &e.feature_id, current)?;
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

pub(super) fn rebase(t: &mut Transaction, s: &str, r: Rebase) -> Result<Value> {
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
    let mut plan = merge_plan(
        t,
        &r.project,
        &r.workspace,
        base,
        current,
        Some(("", 100)),
        MergeMode::Stage,
    )?;
    let mut required = plan.conflict_keys.clone();
    for e in r.resolutions {
        if !required.remove(&(e.dataset.clone(), e.feature_id.clone())) {
            return Err(Error::new(409, "resolutions must match conflicts exactly"));
        }
        let value = e.feature.map(|f| normalize(f, &e.feature_id)).transpose()?;
        if let Some(v) = &value {
            validate_dataset_geometry(t, &r.project, &e.dataset, v)?;
        }
        if value != at_revision(t, &r.project, &e.dataset, &e.feature_id, current)? {
            t.stage_resolution(
                &e.dataset,
                &e.feature_id,
                &value.as_ref().map(|v| json!(v).to_string()),
            )?;
            plan.change_count += 1;
        }
    }
    if !required.is_empty() {
        return Err(conflicts(current, version, plan));
    }
    t.clear_deltas(&r.project, &r.workspace)?;
    t.replace_deltas_with_merge(&r.project, &r.workspace)?;
    t.advance_base(&r.project, &r.workspace, current)?;
    let v = bump(t, &r.project, &r.workspace, "open")?;
    audit(
        t,
        &r.project,
        s,
        "rebase",
        json!({"workspace":r.workspace,"base_revision":current,"version":v}),
    )?;
    Ok(json!({"base_revision":current,"version":v,"changes":plan.change_count}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Restore {
    project: String,
    revision: i64,
}
pub(super) fn restore(t: &mut Transaction, s: &str, r: Restore) -> Result<Value> {
    head(t, &r.project, true)?;
    membership(t, &r.project, s, true)?;
    t.commit_exists(&r.project, r.revision)?
        .ok_or_else(missing)?;
    let result = new_workspace(t, s, &r.project, r.revision)?;
    let w = result["workspace"].as_str().ok_or_else(bad)?;
    t.restore_deltas(&r.project, r.revision, w)?;
    audit(
        t,
        &r.project,
        s,
        "restore",
        json!({"workspace":w,"reverts_revision":r.revision}),
    )?;
    Ok(result)
}

#[derive(Clone)]
pub(super) struct Delta {
    dataset: String,
    key: String,
    value: Option<Stored>,
    resolved_head: Option<i64>,
    resolution_stale: bool,
}
