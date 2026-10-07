use super::*;

struct Workspace {
    id: String,
    base: i64,
    version: i64,
    columns: Vec<Column>,
    schema_head: Option<i64>,
}
fn workspace(t: &mut Transaction<'_>, d: &Dataset, s: &Scope, w: &str) -> Result<Workspace> {
    id(w)?;
    let row=t.query_opt("SELECT base_revision,version,columns::text,resolved_schema_head,closed FROM _geoledger_center.collab_workspaces WHERE epoch=$1::text::uuid AND id=$2::text::uuid AND owner=$3 FOR UPDATE",&[&d.epoch,&w,&s.subject])?.ok_or_else(missing)?;
    if row.get::<_, bool>(4) {
        return Err(stale());
    }
    Ok(Workspace {
        id: w.into(),
        base: row.get(0),
        version: row.get(1),
        columns: parse(&row.get::<_, String>(2))?,
        schema_head: row.get(3),
    })
}
fn expect_version(w: &Workspace, version: i64) -> Result<()> {
    if w.version != version || !(0..i64::from(i32::MAX)).contains(&version) {
        Err(stale())
    } else {
        Ok(())
    }
}
fn ids(t: &mut Transaction<'_>, w: &Workspace) -> Result<Value> {
    let rows=t.query("SELECT client_id,id FROM _geoledger_center.collab_ids WHERE workspace=$1::text::uuid ORDER BY client_id",&[&w.id])?;
    Ok(Value::Object(
        rows.into_iter()
            .map(|r| (r.get::<_, String>(0), json!(r.get::<_, String>(1))))
            .collect(),
    ))
}
fn result(t: &mut Transaction<'_>, w: &Workspace) -> Result<Value> {
    Ok(
        json!({"workspace":w.id,"version":w.version+1,"base_revision":w.base.to_string(),"ids":ids(t,w)?}),
    )
}
fn project(mut value: Option<Stored>, columns: &[Column]) -> Option<Stored> {
    if let Some(v) = value.as_mut() {
        v.properties
            .retain(|k, _| columns.iter().any(|c| c.name == *k));
    }
    value
}
fn draft_at(
    t: &mut Transaction<'_>,
    d: &Dataset,
    w: &Workspace,
    key: &str,
) -> Result<Option<Stored>> {
    if let Some(row)=t.query_opt("SELECT value::text FROM _geoledger_center.collab_drafts WHERE workspace=$1::text::uuid AND id=$2",&[&w.id,&key])?{row.get::<_,Option<String>>(0).map(|s|parse(&s)).transpose()}else{at(t,d,key,w.base)}
}
fn store_draft(
    t: &mut Transaction<'_>,
    w: &Workspace,
    key: &str,
    value: Option<&Stored>,
    head: Option<i64>,
) -> Result<()> {
    let value = value.map(|v| json!(v).to_string());
    t.execute("INSERT INTO _geoledger_center.collab_drafts(workspace,id,value,resolved_head) VALUES($1::text::uuid,$2,$3::text::jsonb,$4) ON CONFLICT(workspace,id) DO UPDATE SET value=excluded.value,resolved_head=excluded.resolved_head",&[&w.id,&key,&value,&head])?;
    Ok(())
}
fn bump(t: &mut Transaction<'_>, w: &Workspace) -> Result<()> {
    t.execute("UPDATE _geoledger_center.collab_workspaces SET version=version+1,columns=$2::text::jsonb,resolved_schema_head=$3 WHERE id=$1::text::uuid",&[&w.id,&json!(w.columns).to_string(),&w.schema_head])?;
    Ok(())
}
pub(super) fn save_delta(
    t: &mut Transaction<'_>,
    d: &Dataset,
    s: &Scope,
    id: &str,
    version: i64,
    operations: Vec<Value>,
) -> Result<Value> {
    if operations.is_empty() || operations.len() > 1000 {
        return Err(Error::new(
            400,
            "upload requires 1..1000 operations per chunk",
        ));
    }
    let mut w = workspace(t, d, s, id)?;
    expect_version(&w, version)?;
    // Any changed draft content invalidates prior human choices.
    t.execute("UPDATE _geoledger_center.collab_drafts SET resolved_head=-1 WHERE workspace=$1::text::uuid AND resolved_head IS NOT NULL",&[&w.id])?;
    if w.schema_head.is_some() {
        w.schema_head = Some(-1);
    }
    for operation in operations {
        let method = operation
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(bad)?;
        if operation.get("schema") == Some(&Value::Bool(true)) {
            if method != "patch" {
                return Err(bad());
            }
            let body = operation
                .get("body")
                .and_then(Value::as_object)
                .ok_or_else(bad)?;
            if let Some(drop) = body.get("drop") {
                for name in drop.as_array().ok_or_else(bad)? {
                    let name = name.as_str().ok_or_else(bad)?;
                    let column = w.columns.iter().find(|c| c.name == name).ok_or_else(bad)?;
                    if !column.editable {
                        return Err(Error::new(422, "column is not editable"));
                    }
                    validate_drop(t, d, name)?;
                    w.columns.retain(|c| c.name != name);
                }
            }
            if let Some(add) = body.get("add") {
                for column in add.as_array().ok_or_else(bad)? {
                    let name = column.get("name").and_then(Value::as_str).ok_or_else(bad)?;
                    let kind = column.get("type").and_then(Value::as_str).ok_or_else(bad)?;
                    if name == d.binding.geometry_column || w.columns.iter().any(|c| c.name == name)
                    {
                        return Err(Error::new(409, "column already exists"));
                    }
                    if schema_at(t, d, w.base)?.iter().any(|c| c.name == name) {
                        return Err(Error::new(
                            422,
                            "replacing a base column is unsupported; use a new column name",
                        ));
                    }
                    w.columns.push(new_column(name, kind)?);
                }
            }
            continue;
        }
        let key = if method == "post" {
            let client = operation
                .get("client_id")
                .and_then(Value::as_str)
                .ok_or_else(bad)?;
            allocate(t, d, &w.id, client, &w.columns)?
        } else {
            operation
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(bad)?
                .to_owned()
        };
        text(&key, 256)?;
        if key == "$schema" {
            return Err(Error::new(
                422,
                "feature ID $schema is reserved for schema conflicts",
            ));
        }
        let value = match method {
            "delete" => None,
            "patch" => {
                let old = draft_at(t, d, &w, &key)?.ok_or_else(missing)?;
                Some(normalize_patch(
                    t,
                    d,
                    &key,
                    old,
                    operation.get("body").ok_or_else(bad)?,
                    &w.columns,
                    false,
                )?)
            }
            "restore" => {
                if at(t, d, &key, w.base)?.is_some() {
                    return Err(Error::new(
                        409,
                        "restore requires a deleted row at the draft base",
                    ));
                }
                let old = restorable(t, d, &key, w.base)?;
                Some(normalize_patch(
                    t,
                    d,
                    &key,
                    old,
                    operation.get("body").ok_or_else(bad)?,
                    &w.columns,
                    false,
                )?)
            }
            "post" => {
                let old = Stored {
                    properties: Map::from_iter([(d.binding.id_column.clone(), json!(key))]),
                    geometry: None,
                };
                Some(normalize_patch(
                    t,
                    d,
                    &key,
                    old,
                    operation.get("body").ok_or_else(bad)?,
                    &w.columns,
                    true,
                )?)
            }
            _ => return Err(bad()),
        };
        let base = project(at(t, d, &key, w.base)?, &w.columns);
        let value = project(value, &w.columns);
        if value == base {
            t.execute("DELETE FROM _geoledger_center.collab_drafts WHERE workspace=$1::text::uuid AND id=$2",&[&w.id,&key])?;
        } else {
            store_draft(t, &w, &key, value.as_ref(), None)?;
        }
    }
    w.columns.sort_by(|a, b| a.name.cmp(&b.name));
    bump(t, &w)?;
    result(t, &w)
}
pub(super) fn draft_page(
    t: &mut Transaction<'_>,
    d: &Dataset,
    s: &Scope,
    id: &str,
    after: Option<String>,
    limit: Option<usize>,
) -> Result<Value> {
    let w = workspace(t, d, s, id)?;
    let context = json!([d.epoch, "draft", w.id, w.version]);
    let after = after_key(after, &context)?;
    let limit = page_limit(limit)?;
    let rows=t.query("WITH candidates AS (SELECT d.id,d.value,m.client_id FROM _geoledger_center.collab_drafts d LEFT JOIN _geoledger_center.collab_ids m ON m.workspace=d.workspace AND m.id=d.id WHERE d.workspace=$1::text::uuid AND d.id>$2 ORDER BY d.id LIMIT $3), sized AS (SELECT *,coalesce(sum(coalesce(octet_length(value::text),0)+octet_length(id)+64) OVER(ORDER BY id ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) bytes FROM candidates) SELECT id,value::text,client_id,(SELECT count(*) FROM candidates) FROM sized WHERE bytes<1048576 ORDER BY id",&[&w.id,&after,&((limit+1) as i64)])?;
    let mut items = Vec::new();
    let mut bytes = 0;
    let mut last = after;
    let mut done = rows
        .first()
        .is_none_or(|r| r.get::<_, i64>(3) as usize == rows.len());
    for row in rows {
        let key: String = row.get(0);
        let native = row
            .get::<_, Option<String>>(1)
            .map(|v| parse::<Stored>(&v))
            .transpose()?;
        let client_id: Option<String> = row.get(2);
        let restore =
            if client_id.is_none() && native.is_some() && at(t, d, &key, w.base)?.is_none() {
                {
                    let original = restorable(t, d, &key, w.base)?;
                    outward(t, &key, Some(&original), &w.columns)?
                }
            } else {
                Value::Null
            };
        let mut feature = outward(t, &key, native.as_ref(), &w.columns)?;
        if client_id.is_some()
            && let Some(native) = &native
            && let Some(properties) = feature.get_mut("properties").and_then(Value::as_object_mut)
        {
            properties.retain(|name, _| native.properties.contains_key(name));
        }
        let value = json!({"id":key,"feature":feature,"client_id":client_id,"restore":restore});
        let size = value.to_string().len();
        if items.len() == limit || (!items.is_empty() && bytes + size > PAGE_BYTES) {
            done = false;
            break;
        }
        last = key;
        bytes += size;
        items.push(value);
    }
    Ok(
        json!({"workspace":w.id,"version":w.version,"base_revision":w.base.to_string(),"changes":items,"schema":w.columns,"ids":ids(t,&w)?,"next_after":if done{None}else{Some(cursor(&context,&last))},"done":done}),
    )
}
fn schema_record(columns: &[Column]) -> Record {
    Record {
        key: "$schema".into(),
        fields: columns
            .iter()
            .map(|c| (c.name.clone(), Cell::Text(json!(c).to_string())))
            .collect(),
    }
}
fn merged_schema(
    t: &mut Transaction<'_>,
    d: &Dataset,
    w: &Workspace,
) -> Result<std::result::Result<Vec<Column>, Value>> {
    let base = schema_at(t, d, w.base)?;
    let remote = schema_at(t, d, d.head)?;
    let conflict = |fields: Vec<String>| json!({"id":"$schema","fields":fields,"base":base,"local":w.columns,"remote":remote});
    if let Some(head) = w.schema_head {
        if head == d.head {
            return Ok(Ok(w.columns.clone()));
        }
        return Ok(Err(conflict(vec!["stale_resolution".into()])));
    }
    let merged = match merge_record(
        Some(&schema_record(&base)),
        Some(&schema_record(&w.columns)),
        Some(&schema_record(&remote)),
    ) {
        Ok(Some(r)) => r
            .fields
            .into_values()
            .map(|v| match v {
                Cell::Text(s) => parse::<Column>(&s),
                _ => Err(bad()),
            })
            .collect::<Result<Vec<_>>>()?,
        Err(fields) => return Ok(Err(conflict(fields))),
        _ => return Err(bad()),
    };
    for col in &base {
        let local_has = w.columns.iter().any(|c| c.name == col.name);
        let remote_has = remote.iter().any(|c| c.name == col.name);
        if !local_has && remote_has {
            let changed:bool=t.query_one("SELECT EXISTS(SELECT 1 FROM (SELECT DISTINCT ON(id) id,value FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND revision>$2 AND revision<=$3 ORDER BY id,revision DESC) r LEFT JOIN LATERAL (SELECT value FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND id=r.id AND revision<=$2 ORDER BY revision DESC LIMIT 1) b ON true WHERE r.value IS NOT NULL AND r.value->'properties'->($4::text) IS DISTINCT FROM b.value->'properties'->($4::text))",&[&d.epoch,&w.base,&d.head,&col.name])?.get(0);
            if changed {
                return Ok(Err(conflict(vec![col.name.clone()])));
            }
        }
        if local_has && !remote_has {
            let changed:bool=t.query_one("SELECT EXISTS(SELECT 1 FROM _geoledger_center.collab_drafts r LEFT JOIN LATERAL (SELECT value FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND id=r.id AND revision<=$2 ORDER BY revision DESC LIMIT 1) b ON true WHERE r.workspace=$3::text::uuid AND r.value IS NOT NULL AND r.value->'properties'->($4::text) IS DISTINCT FROM b.value->'properties'->($4::text))",&[&d.epoch,&w.base,&w.id,&col.name])?.get(0);
            if changed {
                return Ok(Err(conflict(vec![col.name.clone()])));
            }
        }
    }
    Ok(Ok(merged))
}
struct Merged {
    key: String,
    base: Option<Stored>,
    local: Option<Stored>,
    remote: Option<Stored>,
    merged: std::result::Result<Option<Stored>, Value>,
}
fn select_fields(merged: &Merged, resolution: &Resolution) -> Result<Option<Stored>> {
    let choice = |path: &str| -> Result<&str> {
        let value = if resolution.choice == "fields" {
            resolution
                .fields
                .get(path)
                .or_else(|| resolution.fields.get("stale_resolution"))
                .map(String::as_str)
                .ok_or_else(bad)?
        } else {
            resolution.choice.as_str()
        };
        if matches!(value, "local" | "remote") {
            Ok(value)
        } else {
            Err(bad())
        }
    };
    let (Some(base), Some(local), Some(remote)) = (&merged.base, &merged.local, &merged.remote)
    else {
        return match choice("*")? {
            "local" => Ok(merged.local.clone()),
            "remote" => Ok(merged.remote.clone()),
            _ => Err(bad()),
        };
    };
    let mut base = base.record(&merged.key);
    let mut local = local.record(&merged.key);
    let remote = remote.record(&merged.key);
    let fields = match merge_record(Some(&base), Some(&local), Some(&remote)) {
        Ok(value) => return value.map(Stored::from_record).transpose(),
        Err(fields) => fields,
    };
    if resolution.choice == "fields"
        && resolution
            .fields
            .keys()
            .any(|key| key != "stale_resolution" && !fields.contains(key))
    {
        return Err(Error::new(
            400,
            "resolution names a field that is not conflicted",
        ));
    }
    for field in fields {
        let selected = if choice(&field)? == "local" {
            local.fields.get(&field)
        } else {
            remote.fields.get(&field)
        }
        .cloned();
        match selected {
            Some(value) => {
                local.fields.insert(field.clone(), value);
            }
            None => {
                local.fields.remove(&field);
            }
        }
        match remote.fields.get(&field) {
            Some(value) => {
                base.fields.insert(field, value.clone());
            }
            None => {
                base.fields.remove(&field);
            }
        }
    }
    merge_record(Some(&base), Some(&local), Some(&remote))
        .map_err(|_| stale())?
        .map(Stored::from_record)
        .transpose()
}
fn merge_one(
    t: &mut Transaction<'_>,
    d: &Dataset,
    w: &Workspace,
    key: String,
    local: Option<Stored>,
    resolved: Option<i64>,
    columns: &[Column],
) -> Result<Merged> {
    let base = project(at(t, d, &key, w.base)?, columns);
    let mut remote = project(at(t, d, &key, d.head)?, columns);
    let mut local = project(local, columns);
    // Once the target schema keeps a deleted column, its historical value is
    // the unchanged side of the row merge. The schema conflict owns that choice.
    if let Some(base) = &base {
        for row in [&mut local, &mut remote].into_iter().flatten() {
            for (key, value) in &base.properties {
                row.properties
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
        }
    }
    let candidate = if let Some(head) = resolved {
        if head == d.head {
            Ok(local.clone())
        } else {
            Err(vec!["stale_resolution".into()])
        }
    } else {
        match merge_record(
            base.as_ref().map(|r| r.record(&key)).as_ref(),
            local.as_ref().map(|r| r.record(&key)).as_ref(),
            remote.as_ref().map(|r| r.record(&key)).as_ref(),
        ) {
            Ok(v) => Ok(v.map(Stored::from_record).transpose()?),
            Err(fields) => Err(fields),
        }
    };
    let merged = match candidate {
        Ok(v) => Ok(v),
        Err(fields) => Err(
            json!({"id":key,"fields":fields,"base":outward(t,&key,base.as_ref(),columns)?,"local":outward(t,&key,local.as_ref(),columns)?,"remote":outward(t,&key,remote.as_ref(),columns)?}),
        ),
    };
    Ok(Merged {
        key,
        base,
        local,
        remote,
        merged,
    })
}
// A draft can contain any number of chunks. Scans retain at most one small row
// batch plus the bounded conflict response; large features are admitted singly.
fn visit_drafts(
    t: &mut Transaction<'_>,
    d: &Dataset,
    w: &Workspace,
    columns: &[Column],
    mut visit: impl FnMut(&mut Transaction<'_>, Merged) -> Result<()>,
) -> Result<()> {
    let mut after = String::new();
    loop {
        let rows=t.query("WITH candidates AS (SELECT id,value,resolved_head FROM _geoledger_center.collab_drafts WHERE workspace=$1::text::uuid AND id>$2 ORDER BY id LIMIT 64), sized AS (SELECT *,coalesce(sum(coalesce(octet_length(value::text),0)+octet_length(id)+64) OVER(ORDER BY id ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) bytes FROM candidates) SELECT id,value::text,resolved_head FROM sized WHERE bytes<1048576 ORDER BY id",&[&w.id,&after])?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let key: String = row.get(0);
            after = key.clone();
            let value = row
                .get::<_, Option<String>>(1)
                .map(|s| parse(&s))
                .transpose()?;
            let merged = merge_one(t, d, w, key, value, row.get(2), columns)?;
            visit(t, merged)?;
        }
    }
    Ok(())
}
fn conflicts(
    t: &mut Transaction<'_>,
    d: &Dataset,
    w: &Workspace,
    after: Option<String>,
    limit: Option<usize>,
) -> Result<Value> {
    let context = json!([d.epoch, "preview", w.id, w.version, d.head.to_string()]);
    let after = after_key(after, &context)?;
    let limit = page_limit(limit)?;
    let mut conflicts = Vec::new();
    let mut total = 0;
    let mut bytes = 0;
    let mut next = None;
    let mut remaining = 0;
    let mut full = false;
    let mut take = |conflict: Value| {
        total += 1;
        let key = conflict["id"].as_str().unwrap_or("");
        if key <= after.as_str() {
            return;
        }
        remaining += 1;
        let size = conflict.to_string().len();
        if !full && conflicts.len() < limit && (conflicts.is_empty() || bytes + size <= PAGE_BYTES)
        {
            bytes += size;
            next = Some(key.to_owned());
            conflicts.push(conflict);
        } else {
            full = true;
        }
    };
    let mut schema_conflict = None;
    let columns = match merged_schema(t, d, w)? {
        Ok(c) => c,
        Err(c) => {
            schema_conflict = Some(c);
            w.columns.clone()
        }
    };
    visit_drafts(t, d, w, &columns, |_, merged| {
        if merged.key.as_str() > "$schema"
            && let Some(conflict) = schema_conflict.take()
        {
            take(conflict);
        }
        if let Err(c) = merged.merged {
            take(c);
        }
        Ok(())
    })?;
    if let Some(conflict) = schema_conflict {
        take(conflict);
    }
    let done = conflicts.len() == remaining;
    Ok(
        json!({"head":d.head.to_string(),"version":w.version,"base_revision":w.base.to_string(),"conflicts":conflicts,"total":total,"next_after":if done{None}else{next.map(|k|cursor(&context,&k))}}),
    )
}
pub(super) fn preview(
    t: &mut Transaction<'_>,
    d: &Dataset,
    s: &Scope,
    id: &str,
    after: Option<String>,
    limit: Option<usize>,
) -> Result<Value> {
    let w = workspace(t, d, s, id)?;
    conflicts(t, d, &w, after, limit)
}
fn ensure_clean(t: &mut Transaction<'_>, d: &Dataset, w: &Workspace) -> Result<Vec<Column>> {
    let found = conflicts(t, d, w, None, Some(20))?;
    if found["total"] != 0 {
        let mut e = Error::new(409, "merge requires conflict resolution");
        for (key, value) in found.as_object().ok_or_else(bad)? {
            e.body[key] = value.clone();
        }
        return Err(e);
    }
    merged_schema(t, d, w)?.map_err(|_| stale())
}
pub(super) fn resolve_draft(
    t: &mut Transaction<'_>,
    d: &Dataset,
    s: &Scope,
    id: &str,
    version: i64,
    head: i64,
    resolutions: Vec<Resolution>,
) -> Result<Value> {
    let mut w = workspace(t, d, s, id)?;
    expect_version(&w, version)?;
    if head != d.head || resolutions.is_empty() || resolutions.len() > 1000 {
        return Err(stale());
    }
    for resolution in resolutions {
        if resolution.id == "$schema" {
            if merged_schema(t, d, &w)?.is_ok() {
                return Err(Error::new(409, "schema is not in conflict"));
            }
            w.columns = match resolution.choice.as_str() {
                "local" => w.columns.clone(),
                "remote" => schema_at(t, d, d.head)?,
                _ => return Err(bad()),
            };
            w.schema_head = Some(head);
            continue;
        }
        let row=t.query_opt("SELECT value::text,resolved_head FROM _geoledger_center.collab_drafts WHERE workspace=$1::text::uuid AND id=$2",&[&w.id,&resolution.id])?.ok_or_else(missing)?;
        let local = row
            .get::<_, Option<String>>(0)
            .map(|s| parse(&s))
            .transpose()?;
        let columns = merged_schema(t, d, &w)?.unwrap_or_else(|_| w.columns.clone());
        let merged = merge_one(t, d, &w, resolution.id.clone(), local, row.get(1), &columns)?;
        if merged.merged.is_ok() {
            return Err(Error::new(409, "feature is not in conflict"));
        }
        let value = match resolution.choice.as_str() {
            "local" | "remote" | "fields" => select_fields(&merged, &resolution)?,
            "custom" => {
                if resolution.feature.is_null() {
                    None
                } else {
                    let value = Stored {
                        properties: Map::from_iter([(
                            d.binding.id_column.clone(),
                            json!(resolution.id),
                        )]),
                        geometry: None,
                    };
                    Some(normalize_patch(
                        t,
                        d,
                        &resolution.id,
                        value,
                        &resolution.feature,
                        &columns,
                        true,
                    )?)
                }
            }
            _ => return Err(bad()),
        };
        store_draft(t, &w, &resolution.id, value.as_ref(), Some(head))?;
    }
    bump(t, &w)?;
    result(t, &w)
}
pub(super) fn rebase_draft(
    t: &mut Transaction<'_>,
    d: &Dataset,
    s: &Scope,
    id: &str,
    version: i64,
    head: i64,
) -> Result<Value> {
    let mut w = workspace(t, d, s, id)?;
    expect_version(&w, version)?;
    if head != d.head {
        return Err(stale());
    }
    let columns = ensure_clean(t, d, &w)?;
    let remote_columns = schema_at(t, d, d.head)?;
    if columns.iter().any(|c| {
        !remote_columns.iter().any(|r| r.name == c.name)
            && w.columns.iter().any(|b| b.name == c.name)
    }) {
        // A retained field deleted remotely needs an explicit local value for
        // every surviving base row before its old schema leaves the draft base.
        t.execute("INSERT INTO _geoledger_center.collab_drafts(workspace,id,value) SELECT $1::text::uuid,b.id,b.value FROM (SELECT DISTINCT ON(id) id,value FROM _geoledger_center.collab_rows WHERE epoch=$2::text::uuid AND revision<=$3 ORDER BY id,revision DESC) b WHERE b.value IS NOT NULL AND (SELECT value IS NOT NULL FROM _geoledger_center.collab_rows WHERE epoch=$2::text::uuid AND id=b.id AND revision<=$4 ORDER BY revision DESC LIMIT 1) ON CONFLICT(workspace,id) DO NOTHING",&[&w.id,&d.epoch,&w.base,&d.head])?;
    }
    visit_drafts(t, d, &w, &columns, |t, merged| {
        let value = merged.merged.map_err(|_| stale())?;
        if value == project(at(t, d, &merged.key, d.head)?, &columns) {
            t.execute("DELETE FROM _geoledger_center.collab_drafts WHERE workspace=$1::text::uuid AND id=$2",&[&w.id,&merged.key])?;
        } else {
            store_draft(t, &w, &merged.key, value.as_ref(), None)?;
        }
        Ok(())
    })?;
    w.base = head;
    w.columns = columns;
    w.schema_head = None;
    t.execute(
        "UPDATE _geoledger_center.collab_workspaces SET base_revision=$2 WHERE id=$1::text::uuid",
        &[&w.id, &head],
    )?;
    bump(t, &w)?;
    result(t, &w)
}
pub(super) fn publish_draft(
    t: &mut Transaction<'_>,
    d: &Dataset,
    s: &Scope,
    id: &str,
    version: i64,
    message: &str,
    host: &impl Host,
) -> Result<Value> {
    if message.len() > 4096 || message.contains('\0') {
        return Err(bad());
    }
    let w = workspace(t, d, s, id)?;
    expect_version(&w, version)?;
    let columns = ensure_clean(t, d, &w)?;
    let before = schema_at(t, d, d.head)?;
    let schema_changed = columns != before;
    t.batch_execute(&format!(
        "LOCK TABLE {} IN SHARE ROW EXCLUSIVE MODE",
        table(&d.binding)
    ))?;
    // Reject a stale schema or any changed-row mutation that bypassed the host.
    let actual = inspect_for_publish(t, &d.binding)?;
    if actual != before {
        return Err(Error::new(
            409,
            "managed table schema changed outside collaboration",
        ));
    }
    let revision = d.head.checked_add(1).ok_or_else(bad)?;
    let mut changed = 0_i64;
    visit_drafts(t, d, &w, &columns, |t, merged| {
        if project(current(t, d, &merged.key, &before)?, &before)
            != project(at(t, d, &merged.key, d.head)?, &before)
        {
            return Err(Error::new(409, "managed row changed outside collaboration"));
        }
        Ok(())
    })?;
    if schema_changed {
        apply_schema(t, d, &before, &columns)?;
        restore_selected_columns(t, d, w.base, &before, &columns)?;
    }
    visit_drafts(t, d, &w, &columns, |t, merged| {
        let value = merged.merged.map_err(|_| stale())?;
        if value == merged.remote {
            return Ok(());
        }
        write_row(
            t,
            d,
            &merged.key,
            value.as_ref(),
            &columns,
            merged.remote.is_some(),
        )?;
        // Capture database defaults, generated values and constraints after the write.
        let final_value = current(t, d, &merged.key, &columns)?;
        let feature = outward(t, &merged.key, final_value.as_ref(), &columns)?;
        t.execute("INSERT INTO _geoledger_center.collab_rows(epoch,id,revision,value,feature) VALUES($1::text::uuid,$2,$3,$4::text::jsonb,$5::text::jsonb)",&[&d.epoch,&merged.key,&revision,&final_value.map(|v|json!(v).to_string()),&if feature.is_null(){None}else{Some(feature.to_string())}])?;
        changed += 1;
        Ok(())
    })?;
    if schema_changed {
        finish_schema(t, d, &before, &columns)?;
    }
    let committed = changed > 0 || schema_changed;
    let revision = if committed { revision } else { d.head };
    if committed {
        if schema_changed {
            capture_schema_rows(t, d, revision, &columns)?;
            t.execute("INSERT INTO _geoledger_center.collab_schemas VALUES($1::text::uuid,$2,$3::text::jsonb)",&[&d.epoch,&revision,&json!(columns).to_string()])?;
        }
        t.execute("INSERT INTO _geoledger_center.collab_commits(epoch,revision,subject,message,changes) VALUES($1::text::uuid,$2,$3,$4,$5)",&[&d.epoch,&revision,&s.subject,&message,&changed])?;
        t.execute(
            "UPDATE _geoledger_center.collab_datasets SET head=$2 WHERE epoch=$1::text::uuid",
            &[&d.epoch, &revision],
        )?;
        host.published(t, s, revision, schema_changed)?;
    }
    t.execute("UPDATE _geoledger_center.collab_workspaces SET closed=true,version=version+1 WHERE id=$1::text::uuid",&[&w.id])?;
    Ok(
        json!({"workspace":w.id,"version":w.version+1,"revision":revision.to_string(),"ids":ids(t,&w)?,"status":if committed{"published"}else{"unchanged"},"changes":changed}),
    )
}
