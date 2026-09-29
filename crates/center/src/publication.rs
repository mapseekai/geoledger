use super::*;

pub(super) struct MergePlan {
    change_count: usize,
    conflicts: Vec<Value>,
    conflict_keys: BTreeSet<(String, String)>,
    truncated: bool,
}
// Only one batch of expanded features is retained. Successful candidates are
// staged in the transaction, not accumulated in a Rust Vec across the workspace.
pub(super) fn merge_plan(
    t: &mut Transaction<'_>,
    p: &str,
    w: &str,
    base: i64,
    current: i64,
    page: Option<(&str, usize)>,
    stage: bool,
) -> Result<MergePlan> {
    if stage {
        t.batch_execute("CREATE TEMP TABLE center_merge(dataset text, feature_id text, before_value jsonb, after_value jsonb) ON COMMIT DROP")?;
    }
    let mut plan = MergePlan {
        change_count: 0,
        conflicts: Vec::new(),
        conflict_keys: BTreeSet::new(),
        truncated: false,
    };
    let mut after = String::new();
    let mut budget = codec::Budget::new();
    // Restrict the workspace prefix before expanding joined historical JSON.
    loop {
        let rows = t.query("WITH batch AS MATERIALIZED (SELECT * FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid AND dataset::text || '/' || feature_id > $5 ORDER BY dataset::text || '/' || feature_id LIMIT 32) SELECT c.dataset::text,c.feature_id,c.properties::text,encode(ST_AsEWKB(c.geom,'XDR'),'hex'),c.resolved_head,c.resolution_stale,b.properties::text,encode(ST_AsEWKB(b.geom,'XDR'),'hex'),o.properties::text,encode(ST_AsEWKB(o.geom,'XDR'),'hex'),ST_AsGeoJSON(c.geom,17,0),ST_AsGeoJSON(b.geom,17,0),ST_AsGeoJSON(o.geom,17,0) FROM batch c LEFT JOIN _geoledger_center.history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) LEFT JOIN _geoledger_center.history o ON o.project=c.project AND o.dataset=c.dataset AND o.feature_id=c.feature_id AND o.valid_from<=$4 AND (o.valid_to IS NULL OR o.valid_to>$4) ORDER BY c.dataset::text || '/' || c.feature_id", &[&p,&w,&base,&current,&after])?;
        if rows.is_empty() {
            break;
        }
        t.geometry_cache.clear();
        let mut staged = Vec::new();
        for row in rows {
            for (hex, geo) in [(3, 10), (7, 11), (9, 12)] {
                if let (Some(hex), Some(geo)) = (
                    row.get::<_, Option<String>>(hex),
                    row.get::<_, Option<String>>(geo),
                ) {
                    t.geometry_cache
                        .insert(hex, serde_json::from_str(&geo).map_err(Error::stored_json)?);
                }
            }
            let d = Delta {
                dataset: row.get(0),
                key: row.get(1),
                value: stored_row(&row, 2, 3)?,
                resolved_head: row.get(4),
                resolution_stale: row.get(5),
            };
            after = format!("{}/{}", d.dataset, d.key);
            let b = stored_row(&row, 6, 7)?;
            let o = stored_row(&row, 8, 9)?;
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
            match merged {
                Ok(value) => {
                    if let Some(v) = &value {
                        validate_candidate(t, &d.key, v)?;
                    }
                    if value != o {
                        plan.change_count += 1;
                        if stage {
                            staged.push(json!({"dataset":d.dataset,"feature_id":d.key,"before_value":o,"after_value":value}));
                        }
                    }
                }
                Err(fields) => {
                    plan.conflict_keys
                        .insert((d.dataset.clone(), d.key.clone()));
                    if let Some((cursor, limit)) = page
                        && after.as_str() > cursor
                    {
                        if plan.conflicts.len() >= limit || plan.truncated {
                            plan.truncated = true;
                            continue;
                        }
                        let mut item = json!({"cursor":after,"dataset":d.dataset,"feature_id":d.key,"fields":fields,"base":geojson(t,&d.key,b.as_ref())?,"current":geojson(t,&d.key,o.as_ref())?,"draft":geojson(t,&d.key,d.value.as_ref())?});
                        if stale_resolution {
                            item["reason"] = json!("stale_resolution");
                            item["resolved_against_revision"] = json!(d.resolved_head);
                        }
                        if budget.take(&item).is_err() {
                            if plan.conflicts.is_empty() {
                                return Err(Error::new(
                                    413,
                                    "conflict exceeds response memory budget",
                                ));
                            }
                            plan.truncated = true;
                        } else {
                            plan.conflicts.push(item);
                        }
                    }
                }
            }
        }
        if !staged.is_empty() {
            let encoded = codec::encode(&staged)?;
            let encoded = std::str::from_utf8(&encoded).map_err(|_| bad())?;
            t.execute("INSERT INTO center_merge SELECT dataset,feature_id,before_value,NULLIF(after_value,'null'::jsonb) FROM jsonb_to_recordset($1::text::jsonb) AS x(dataset text,feature_id text,before_value jsonb,after_value jsonb)",&[&encoded])?;
        }
    }
    t.geometry_cache.clear();
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
pub(super) fn list_conflicts(t: &mut Transaction<'_>, s: &str, r: Diff) -> Result<Value> {
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    let current = head(t, &r.project, false)?;
    let plan = merge_plan(
        t,
        &r.project,
        &r.workspace,
        base,
        current,
        Some((&r.after, r.limit as usize)),
        false,
    )?;
    let next = plan.conflicts.last().map(|v| v["cursor"].clone());
    Ok(
        json!({"head":current,"version":version,"conflicts":plan.conflicts,"total":plan.conflict_keys.len(),"next_after":next,"truncated":plan.truncated}),
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
pub(super) fn publish(t: &mut Transaction<'_>, s: &str, r: Publish) -> Result<Value> {
    text(&r.message, 2048)?;
    let current = head(t, &r.project, true)?;
    membership(t, &r.project, s, true)?;
    workspace(t, s, &r.project, &r.workspace, None)?;
    let request_id = r.request_id.to_string();
    let payload = serde_json::to_value(&r).map_err(|_| bad())?;
    if let Some(row)=t.query_opt("SELECT payload::text,result::text FROM _geoledger_center.idempotency WHERE project=$1::text::uuid AND subject=$2 AND request_id=$3::text::uuid",&[&r.project,&s,&request_id])? {
        let previous:Value=codec::stored(&row.get::<_,String>(0))?;
        if previous!=payload {return Err(Error::new(409,"request_id payload mismatch"));}
        return codec::stored(&row.get::<_,String>(1));
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
        true,
    )?;
    if !plan.conflict_keys.is_empty() {
        return Err(conflicts(current, version, plan));
    }
    let revision = current.checked_add(1).ok_or_else(bad)?;
    t.execute("INSERT INTO _geoledger_center.commits(project,revision,workspace,subject,message) VALUES($1::text::uuid,$2,$3::text::uuid,$4,$5)",&[&r.project,&revision,&r.workspace,&s,&r.message])?;
    // Set-based writes use the already validated, three-way merged candidates.
    t.execute("INSERT INTO _geoledger_center.commit_changes SELECT $1::text::uuid,$2,dataset::uuid,feature_id,NULLIF(before_value,'null'::jsonb),after_value FROM center_merge", &[&r.project,&revision])?;
    t.execute("UPDATE _geoledger_center.history h SET valid_to=$2 FROM center_merge m WHERE h.project=$1::text::uuid AND h.dataset=m.dataset::uuid AND h.feature_id=m.feature_id AND h.valid_to IS NULL", &[&r.project,&revision])?;
    t.execute("INSERT INTO _geoledger_center.history(project,dataset,feature_id,valid_from,properties,geom) SELECT $1::text::uuid,dataset::uuid,feature_id,$2,after_value->'properties',ST_GeomFromEWKB(decode(after_value->>'geometry','hex')) FROM center_merge", &[&r.project,&revision])?;
    t.execute("INSERT INTO _geoledger_center.features SELECT $1::text::uuid,dataset::uuid,feature_id,after_value->'properties',ST_GeomFromEWKB(decode(after_value->>'geometry','hex')) FROM center_merge WHERE after_value IS NOT NULL ON CONFLICT(project,dataset,feature_id) DO UPDATE SET properties=excluded.properties,geom=excluded.geom", &[&r.project])?;
    t.execute("DELETE FROM _geoledger_center.features f USING center_merge m WHERE f.project=$1::text::uuid AND f.dataset=m.dataset::uuid AND f.feature_id=m.feature_id AND m.after_value IS NULL", &[&r.project])?;
    t.execute(
        "UPDATE _geoledger_center.projects SET head=$2 WHERE id=$1::text::uuid",
        &[&r.project, &revision],
    )?;
    let v = bump(t, &r.project, &r.workspace, "published")?;
    let result = json!({"revision":revision,"workspace":r.workspace,"version":v,"status":"published","changes":plan.change_count});
    t.execute("INSERT INTO _geoledger_center.idempotency VALUES($1::text::uuid,$2,$3::text::uuid,$4::text::jsonb,$5::text::jsonb)",&[&r.project,&s,&request_id,&payload.to_string(),&result.to_string()])?;
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
pub(super) fn resolve(t: &mut Transaction<'_>, s: &str, r: Rebase) -> Result<Value> {
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
    let plan = merge_plan(t, &r.project, &r.workspace, base, current, None, false)?;
    let mut remaining = plan.conflict_keys;
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

pub(super) fn rebase(t: &mut Transaction<'_>, s: &str, r: Rebase) -> Result<Value> {
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
        true,
    )?;
    let mut required = plan.conflict_keys.clone();
    for e in r.resolutions {
        if !required.remove(&(e.dataset.clone(), e.feature_id.clone())) {
            return Err(Error::new(409, "resolutions must match conflicts exactly"));
        }
        let value = e
            .feature
            .map(|f| normalize(t, f, &e.feature_id))
            .transpose()?;
        if value != at_revision(t, &r.project, &e.dataset, &e.feature_id, current)? {
            t.execute("INSERT INTO center_merge(dataset,feature_id,after_value) VALUES($1,$2,$3::text::jsonb)", &[&e.dataset,&e.feature_id,&value.as_ref().map(|v| json!(v).to_string())])?;
            plan.change_count += 1;
        }
    }
    if !required.is_empty() {
        return Err(conflicts(current, version, plan));
    }
    t.execute("DELETE FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid",&[&r.project,&r.workspace])?;
    t.execute("INSERT INTO _geoledger_center.workspace_changes(project,workspace,dataset,feature_id,properties,geom) SELECT $1::text::uuid,$2::text::uuid,dataset::uuid,feature_id,after_value->'properties',ST_GeomFromEWKB(decode(after_value->>'geometry','hex')) FROM center_merge", &[&r.project,&r.workspace])?;
    t.execute("UPDATE _geoledger_center.workspaces SET base_revision=$3 WHERE project=$1::text::uuid AND id=$2::text::uuid",&[&r.project,&r.workspace,&current])?;
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
pub(super) fn restore(t: &mut Transaction<'_>, s: &str, r: Restore) -> Result<Value> {
    membership(t, &r.project, s, true)?;
    t.query_opt(
        "SELECT 1 FROM _geoledger_center.commits WHERE project=$1::text::uuid AND revision=$2",
        &[&r.project, &r.revision],
    )?
    .ok_or_else(missing)?;
    let result = new_workspace(t, s, &r.project, r.revision)?;
    let w = result["workspace"].as_str().ok_or_else(bad)?;
    let count: i64 = t.query_one("SELECT count(*) FROM _geoledger_center.commit_changes WHERE project=$1::text::uuid AND revision=$2", &[&r.project,&r.revision])?.get(0);
    if count > 1000 {
        return Err(Error::new(413, "restore exceeds workspace limit"));
    }
    t.execute("INSERT INTO _geoledger_center.workspace_changes(project,workspace,dataset,feature_id,properties,geom) SELECT project,$3::text::uuid,dataset,feature_id,before_value->'properties',ST_GeomFromEWKB(decode(before_value->>'geometry','hex')) FROM _geoledger_center.commit_changes WHERE project=$1::text::uuid AND revision=$2", &[&r.project,&r.revision,&w])?;
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
