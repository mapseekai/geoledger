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
pub(super) fn features(t: &mut Transaction<'_>, s: &str, r: Features) -> Result<Value> {
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
    let mut budget = codec::Budget::new();
    let mut after = r.after.clone();
    while values.len() < r.limit as usize {
        let batch_limit = (r.limit - values.len() as i64).min(32);
        let rows=t.query("WITH overlay AS (SELECT h.feature_id,h.properties,h.geom FROM _geoledger_center.history h WHERE h.project=$1::text::uuid AND h.dataset=$2::text::uuid AND h.valid_from<=$3 AND (h.valid_to IS NULL OR h.valid_to>$3) AND NOT EXISTS(SELECT 1 FROM _geoledger_center.workspace_changes c WHERE c.project=h.project AND c.dataset=h.dataset AND c.feature_id=h.feature_id AND c.workspace=$4::text::uuid) UNION ALL SELECT feature_id,properties,geom FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND dataset=$2::text::uuid AND workspace=$4::text::uuid) SELECT feature_id,properties::text,ST_AsGeoJSON(geom,17,0) FROM overlay WHERE properties IS NOT NULL AND feature_id>$5 AND ($6::text IS NULL OR feature_id=$6) AND (NOT $7 OR ST_Intersects(geom,ST_MakeEnvelope($8,$9,$10,$11,4326))) ORDER BY feature_id LIMIT $12",&[&r.project,&r.dataset,&revision,&r.workspace,&after,&r.feature_id,&r.bbox.is_some(),&bbox[0],&bbox[1],&bbox[2],&bbox[3],&batch_limit])?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row.get(0);
            let properties: Value = codec::stored(&row.get::<_, String>(1))?;
            let geometry: Value = row
                .get::<_, Option<String>>(2)
                .map(|s| serde_json::from_str(&s))
                .transpose()
                .map_err(Error::stored_json)?
                .unwrap_or(Value::Null);
            let item = json!({"type":"Feature","id":row.get::<_,String>(0),"properties":properties,"geometry":geometry});
            budget.take(&item)?;
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
pub(super) fn diff(t: &mut Transaction<'_>, s: &str, r: Diff) -> Result<Value> {
    let (base, version, _) = workspace(t, s, &r.project, &r.workspace, None)?;
    Page {
        after: r.after.clone(),
        limit: r.limit,
    }
    .check()?;
    let mut result = Vec::new();
    let mut after = r.after;
    let mut budget = codec::Budget::new();
    while result.len() < r.limit as usize {
        let limit = (r.limit - result.len() as i64).min(32);
        let rows = t.query("WITH batch AS MATERIALIZED (SELECT * FROM _geoledger_center.workspace_changes WHERE project=$1::text::uuid AND workspace=$2::text::uuid AND dataset::text || '/' || feature_id>$4 ORDER BY dataset::text || '/' || feature_id LIMIT $5) SELECT c.dataset::text,c.feature_id,c.properties::text,ST_AsGeoJSON(c.geom,17,0),b.properties::text,ST_AsGeoJSON(b.geom,17,0) FROM batch c LEFT JOIN _geoledger_center.history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) ORDER BY c.dataset::text || '/' || c.feature_id", &[&r.project,&r.workspace,&base,&after,&limit])?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let dataset: String = row.get(0);
            let key: String = row.get(1);
            after = format!("{dataset}/{key}");
            let item = json!({"cursor":after,"dataset":dataset,"feature_id":key,"base":feature_row(&row,&key,4,5)?,"draft":feature_row(&row,&key,2,3)?});
            budget.take(&item)?;
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
pub(super) fn history(t: &mut Transaction<'_>, s: &str, r: History) -> Result<Value> {
    membership(t, &r.project, s, false)?;
    if r.after < 0 || !(1..=1000).contains(&r.limit) {
        return Err(bad());
    }
    let rows=t.query("SELECT revision,subject,message,created_at::text FROM _geoledger_center.commits WHERE project=$1::text::uuid AND revision>$2 ORDER BY revision LIMIT $3",&[&r.project,&r.after,&r.limit])?;
    Ok(json!(rows.iter().map(|x|json!({"revision":x.get::<_,i64>(0),"subject":x.get::<_,String>(1),"message":x.get::<_,String>(2),"created_at":x.get::<_,String>(3)})).collect::<Vec<_>>()))
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
pub(super) fn commit_detail(t: &mut Transaction<'_>, s: &str, r: Commit) -> Result<Value> {
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
    let mut out = Vec::new();
    let mut budget = codec::Budget::new();
    let mut after = r.after;
    while out.len() < r.limit as usize {
        let limit = (r.limit - out.len() as i64).min(32);
        let rows = t.query("SELECT dataset::text,feature_id,(before_value->'properties')::text,ST_AsGeoJSON(ST_GeomFromEWKB(decode(before_value->>'geometry','hex')),17,0),(after_value->'properties')::text,ST_AsGeoJSON(ST_GeomFromEWKB(decode(after_value->>'geometry','hex')),17,0) FROM _geoledger_center.commit_changes WHERE project=$1::text::uuid AND revision=$2 AND dataset::text || '/' || feature_id>$3 ORDER BY dataset::text || '/' || feature_id LIMIT $4", &[&r.project,&r.revision,&after,&limit])?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            let d: String = row.get(0);
            let key: String = row.get(1);
            after = format!("{d}/{key}");
            let item = json!({"cursor":after,"dataset":d,"feature_id":key,"before":feature_row(&row,&key,2,3)?,"after":feature_row(&row,&key,4,5)?});
            budget.take(&item)?;
            out.push(item);
        }
    }
    Ok(json!({"revision":r.revision,"changes":out}))
}

fn feature_row(row: &Row, key: &str, properties: usize, geometry: usize) -> Result<Value> {
    let Some(properties) = row.get::<_, Option<String>>(properties) else {
        return Ok(Value::Null);
    };
    let properties: Value = codec::stored(&properties)?;
    let geometry: Value = row
        .get::<_, Option<String>>(geometry)
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(Error::stored_json)?
        .unwrap_or(Value::Null);
    Ok(json!({"type":"Feature","id":key,"properties":properties,"geometry":geometry}))
}
