//! Opt-in small-batch CRUD benchmark over an already populated disposable dataset.
use geoledger_client::{Client, Edit, FeatureQuery};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    time::{Duration, Instant},
};
type Failure = Box<dyn std::error::Error + Send + Sync>;

#[derive(Deserialize)]
struct Manifest {
    project: String,
    dataset: String,
    count: u64,
    seed_ids: Vec<String>,
    update_property: Option<String>,
    #[serde(default = "geometry_updates")]
    update_geometry: bool,
}
fn geometry_updates() -> bool {
    true
}
fn summary(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    if samples.is_empty() {
        return json!({"count":0});
    }
    let percentile =
        |p: f64| samples[((samples.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    json!({"count":samples.len(),"p50_ms":percentile(0.5),"p95_ms":percentile(0.95),"p99_ms":percentile(0.99),"max_ms":samples.last(),"mean_ms":samples.iter().sum::<f64>()/samples.len() as f64})
}
fn clean(mut feature: Value) -> Value {
    fn geometry_numbers(value: &mut Value) {
        match value {
            Value::Number(n) => {
                if let Some(n) = n.as_f64() {
                    *value = json!(n);
                }
            }
            Value::Array(values) => values.iter_mut().for_each(geometry_numbers),
            Value::Object(values) => values.values_mut().for_each(geometry_numbers),
            _ => {}
        }
    }
    if let Some(object) = feature.as_object_mut() {
        object.retain(|key, _| ["type", "id", "properties", "geometry"].contains(&key.as_str()));
        if let Some(geometry) = object.get_mut("geometry") {
            geometry_numbers(geometry);
        }
    }
    feature
}
fn change(feature: &mut Value, key: Option<&str>) -> Result<(), Failure> {
    let properties = feature["properties"]
        .as_object_mut()
        .ok_or("object properties required")?;
    let key = match key {
        Some(key) => key.to_owned(),
        None => properties
            .iter()
            .find(|(_, value)| value.is_string() || value.is_object())
            .map(|(key, _)| key.clone())
            .ok_or("set update_property to a mutable string or JSON object property")?,
    };
    match properties
        .get_mut(&key)
        .ok_or("update_property is absent")?
    {
        Value::String(text) => text.push_str("-incremental"),
        Value::Object(object) => {
            object.insert("incremental_benchmark".into(), json!(true));
        }
        _ => return Err("update_property must contain a string or JSON object".into()),
    }
    Ok(())
}
fn shift_geometry(feature: &mut Value) -> Result<(), Failure> {
    fn positions(
        value: &mut Value,
        visit: &mut impl FnMut(&mut [Value]) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let values = value.as_array_mut().ok_or("coordinates must be arrays")?;
        if values.first().is_some_and(Value::is_number) {
            if !(2..=3).contains(&values.len()) {
                return Err("expected XY or XYZ position".into());
            }
            visit(values)
        } else {
            for value in values {
                positions(value, visit)?;
            }
            Ok(())
        }
    }
    let coords = feature["geometry"]
        .get_mut("coordinates")
        .ok_or("typed geometry coordinates required")?;
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    positions(coords, &mut |values| {
        for axis in 0..2 {
            let value = values[axis]
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or("finite XY required")?;
            minimum[axis] = minimum[axis].min(value);
            maximum[axis] = maximum[axis].max(value);
        }
        Ok(())
    })?;
    let mut delta = [0.0; 2];
    for (axis, bound) in [180.0, 90.0].into_iter().enumerate() {
        if minimum[axis] < -bound || maximum[axis] > bound || !minimum[axis].is_finite() {
            return Err("geometry requires nonempty longitude/latitude coordinates".into());
        }
        delta[axis] = if maximum[axis] <= bound - 1e-7 {
            1e-7
        } else if minimum[axis] >= -bound + 1e-7 {
            -1e-7
        } else {
            return Err("geometry spans both coordinate bounds; cannot translate".into());
        };
    }
    positions(coords, &mut |values| {
        for axis in 0..2 {
            values[axis] = json!(values[axis].as_f64().ok_or("numeric XY required")? + delta[axis]);
        }
        Ok(())
    })
}
async fn read_range(
    client: &Client,
    manifest: &Manifest,
    after_key: &str,
    through_key: &str,
    revision: i64,
) -> Result<Vec<Value>, Failure> {
    let mut result = Vec::new();
    let mut after = after_key.to_owned();
    loop {
        let page = client
            .features(
                &manifest.project,
                &manifest.dataset,
                FeatureQuery {
                    revision: Some(revision),
                    after: after.clone(),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(page.revision, revision);
        let mut finished = false;
        for feature in page.features {
            let id = feature["id"].as_str().ok_or("feature ID")?;
            if id > through_key {
                finished = true;
                break;
            }
            assert!(
                id > after.as_str(),
                "page contains a nonadvancing feature ID"
            );
            finished = id == through_key;
            result.push(clean(feature));
            if finished {
                break;
            }
        }
        if finished {
            break;
        }
        match page.next_after {
            Some(cursor) if cursor > after => after = cursor,
            None => break,
            _ => return Err("pagination did not advance".into()),
        }
    }
    Ok(result)
}
fn checkpoint(path: &str, report: &Value) -> Result<(), Failure> {
    let temporary = format!("{path}.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(report)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
#[tokio::main]
async fn main() -> Result<(), Failure> {
    if std::env::var("GL_BENCH_DISPOSABLE").as_deref() != Ok("1") {
        return Err("set GL_BENCH_DISPOSABLE=1 for an isolated test server".into());
    }
    let args: Vec<String> = std::env::args().collect();
    let manifest: Manifest = serde_json::from_reader(File::open(
        args.get(1)
            .ok_or("usage: incremental MANIFEST REPORT [ROUNDS]")?,
    )?)?;
    let report_path = args.get(2).ok_or("report path required")?;
    let rounds: usize = args
        .get(3)
        .cloned()
        .or_else(|| std::env::var("GL_BENCH_ROUNDS").ok())
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(10);
    if !(3..=100).contains(&rounds) || manifest.seed_ids.is_empty() || manifest.count < 500 {
        return Err(
            "requires 3..100 rounds, at least one seed ID, and 500 background features".into(),
        );
    }
    let token: Value = serde_json::from_reader(File::open(std::env::var("GL_TOKEN_FILE")?)?)?;
    let client = Client::connect_with_timeout(
        std::env::var("GL_ENDPOINT")?,
        token[0]["token"].as_str().ok_or("token")?,
        Duration::from_secs(600),
    )
    .await?;
    let mut seeds = Vec::new();
    for id in &manifest.seed_ids {
        let page = client
            .execute(
                "features",
                json!({"project":manifest.project,"dataset":manifest.dataset,"feature_id":id}),
            )
            .await?;
        let feature = page["features"]
            .as_array()
            .and_then(|v| v.first())
            .ok_or("seed missing")?;
        let feature = clean(feature.clone());
        let mut changed = feature.clone();
        change(&mut changed, manifest.update_property.as_deref())?;
        if manifest.update_geometry {
            shift_geometry(&mut changed)?;
        }
        if changed == feature {
            return Err("update must change the seed value".into());
        }
        seeds.push(feature);
    }
    let mut report = json!({"project":manifest.project,"dataset":manifest.dataset,"background_features":manifest.count,"update_geometry":manifest.update_geometry,"update_mode":if manifest.update_geometry {"properties_and_geometry"} else {"properties"},"rounds":rounds,"concurrency":1,"server":client.info().await?,"timeout_seconds":600,"samples":[],"success":false});
    checkpoint(report_path, &report)?;
    let started = Instant::now();
    let mut metrics: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for workload in ["added", "existing"] {
        for size in [10usize, 100, 500] {
            for round in 0..rounds {
                let initial_revision = client.project(&manifest.project).await?.head;
                let (after_key, through_key, originals) = if workload == "added" {
                    let prefix = format!("incremental-{}-", uuid::Uuid::new_v4());
                    let rows: Vec<Value> = (0..size)
                        .map(|index| {
                            let mut feature = seeds[index % seeds.len()].clone();
                            feature["id"] = json!(format!("{prefix}{index:06}"));
                            feature
                        })
                        .collect();
                    (prefix.clone(), format!("{prefix}999999"), rows)
                } else {
                    let first = (manifest.count - size as u64) / 2 + 1;
                    let after = format!("{:012}", first - 1);
                    let through = format!("{:012}", first + size as u64 - 1);
                    let rows =
                        read_range(&client, &manifest, &after, &through, initial_revision).await?;
                    assert_eq!(rows.len(), size, "existing background range is incomplete");
                    for (index, feature) in rows.iter().enumerate() {
                        assert_eq!(feature["id"], format!("{:012}", first + index as u64));
                    }
                    (after, through, rows)
                };
                let mut updates = originals.clone();
                for feature in &mut updates {
                    change(feature, manifest.update_property.as_deref())?;
                    if manifest.update_geometry {
                        shift_geometry(feature)?;
                    }
                }
                let mut previous_revision = Some(initial_revision);
                let mut previous_features = if workload == "existing" {
                    originals.clone()
                } else {
                    Vec::new()
                };
                let empty = Vec::new();
                let operations = if workload == "added" {
                    [
                        ("add", &originals),
                        ("update", &updates),
                        ("delete", &empty),
                    ]
                } else {
                    [
                        ("update", &updates),
                        ("delete", &empty),
                        ("restore", &originals),
                    ]
                };
                for (operation, features) in operations {
                    let mut workspace = client.create_workspace(&manifest.project).await?;
                    let base_revision = workspace.info().base_revision;
                    if let Some(revision) = previous_revision {
                        assert_eq!(base_revision, revision);
                    }
                    let before = previous_features.clone();
                    let edits: Vec<Edit> = originals
                        .iter()
                        .enumerate()
                        .map(|(index, feature)| {
                            Ok(Edit {
                                dataset: manifest.dataset.clone(),
                                feature_id: feature["id"].as_str().ok_or("feature ID")?.into(),
                                feature: if operation == "delete" {
                                    None
                                } else {
                                    Some(features[index].clone())
                                },
                            })
                        })
                        .collect::<Result<_, Failure>>()?;
                    let save_clock = Instant::now();
                    let mut rpc_ms = Vec::new();
                    let mut saved_count = 0;
                    let mut warnings = 0;
                    for batch in edits.chunks(100) {
                        let old_version = workspace.info().version;
                        let clock = Instant::now();
                        let saved = workspace.save_batch(batch).await?;
                        rpc_ms.push(clock.elapsed().as_secs_f64() * 1000.0);
                        saved_count += batch.len();
                        warnings += saved.warnings.len();
                        assert_eq!(saved.version, old_version + 1);
                        assert_eq!(workspace.info().version, saved.version);
                        assert_eq!(saved.changes, saved_count as i64);
                    }
                    let save_ms = save_clock.elapsed().as_secs_f64() * 1000.0;
                    let clock = Instant::now();
                    let receipt = workspace.publish("incremental benchmark").await?;
                    let publish_ms = clock.elapsed().as_secs_f64() * 1000.0;
                    assert_eq!(receipt.changes, size as u64);
                    assert!(receipt.revision > base_revision);
                    let publication = workspace
                        .pending_publication()
                        .ok_or("pending request missing")?
                        .clone();
                    let clock = Instant::now();
                    assert_eq!(receipt, client.publish(&publication).await?);
                    let replay_ms = clock.elapsed().as_secs_f64() * 1000.0;
                    let clock = Instant::now();
                    assert_eq!(
                        read_range(
                            &client,
                            &manifest,
                            &after_key,
                            &through_key,
                            receipt.revision
                        )
                        .await?,
                        *features,
                        "published readback"
                    );
                    assert_eq!(
                        read_range(&client, &manifest, &after_key, &through_key, base_revision)
                            .await?,
                        before,
                        "historical visibility"
                    );
                    let readback_ms = clock.elapsed().as_secs_f64() * 1000.0;
                    previous_revision = Some(receipt.revision);
                    previous_features = features.clone();
                    for (metric, value) in [
                        ("save", save_ms),
                        ("publish", publish_ms),
                        ("replay", replay_ms),
                        ("readback", readback_ms),
                    ] {
                        metrics
                            .entry(format!("{workload}/{size}/{operation}/{metric}"))
                            .or_default()
                            .push(value);
                    }
                    let sample = json!({"workload":workload,"size":size,"round":round,"operation":operation,"workspace":workspace.id(),"revision":receipt.revision,"save_ms":save_ms,"save_rpc_ms":rpc_ms,"publish_ms":publish_ms,"replay_ms":replay_ms,"readback_ms":readback_ms,"warnings":warnings,"verified":true});
                    println!("{sample}");
                    report["samples"]
                        .as_array_mut()
                        .ok_or("report samples")?
                        .push(sample);
                    report["metrics"] = serde_json::to_value(
                        metrics
                            .iter()
                            .map(|(key, values)| (key.clone(), summary(values.clone())))
                            .collect::<BTreeMap<_, _>>(),
                    )?;
                    report["seconds"] = json!(started.elapsed().as_secs_f64());
                    checkpoint(report_path, &report)?;
                }
                let initial_features = if workload == "existing" {
                    originals
                } else {
                    Vec::new()
                };
                assert_eq!(
                    read_range(
                        &client,
                        &manifest,
                        &after_key,
                        &through_key,
                        initial_revision
                    )
                    .await?,
                    initial_features,
                    "initial snapshot after complete cycle"
                );
            }
        }
    }
    report["success"] = json!(true);
    checkpoint(report_path, &report)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_translation_preserves_z_and_ring_closure() -> Result<(), Failure> {
        let mut feature = json!({"geometry":{"type":"Polygon","coordinates":[[[180.0,90.0,8],[179.0,89.0,9],[178.0,89.0,10],[180.0,90.0,8]]]}});
        shift_geometry(&mut feature)?;
        let ring = feature["geometry"]["coordinates"][0]
            .as_array()
            .ok_or("ring")?;
        assert_eq!(ring.first(), ring.last());
        assert_eq!(ring[0][2], 8);
        assert_eq!(ring[1][2], 9);
        assert_eq!(ring[0][0], json!(180.0 - 1e-7));
        assert_eq!(ring[0][1], json!(90.0 - 1e-7));
        assert_eq!(feature["geometry"]["type"], "Polygon");
        let mut invalid = json!({"geometry":{"coordinates":[181,0]}});
        assert!(shift_geometry(&mut invalid).is_err());
        let parsed: Manifest = serde_json::from_value(
            json!({"project":"p","dataset":"d","count":1,"seed_ids":["1"]}),
        )?;
        assert!(parsed.update_geometry);
        Ok(())
    }
    #[test]
    fn preserves_schema_geometry_and_large_numbers() {
        let source = json!({"type":"Feature","id":"a","properties":{"label":"road","n":18446744073709551615u64},"geometry":{"type":"Point","coordinates":[1.25,2.5,3.75]},"revision":7});
        let mut feature = clean(source.clone());
        change(&mut feature, Some("label")).unwrap();
        assert_eq!(feature["geometry"], source["geometry"]);
        assert_eq!(feature["properties"]["n"].as_u64(), Some(u64::MAX));
        assert_eq!(feature["properties"].as_object().unwrap().len(), 2);
        assert_eq!(feature["properties"]["label"], "road-incremental");
        assert!(feature.get("revision").is_none());
        assert!(change(&mut feature, Some("missing")).is_err());
        assert!(change(&mut feature, Some("n")).is_err());
        let s = summary((1..=100).map(f64::from).collect());
        assert_eq!(s["p95_ms"], 95.0);
        assert_eq!(summary(vec![])["count"], 0);
    }
}
