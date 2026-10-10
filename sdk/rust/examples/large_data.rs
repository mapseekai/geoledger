//! Opt-in real-file RPC benchmark. Run only against a disposable server.
#[path = "large_data/input.rs"]
mod input;
use geoledger_client::{Client, Edit, FeatureQuery};
use serde_json::{Value, json};
use sha2::Digest;
use std::{
    collections::BTreeMap,
    fs::File,
    io::BufReader,
    time::{Duration, Instant},
};
type Failure = Box<dyn std::error::Error + Send + Sync>;

fn summary(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    if samples.is_empty() {
        return json!({"count":0});
    }
    let percentile =
        |p: f64| samples[((samples.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    json!({"count":samples.len(),"p50_ms":percentile(0.5),"p95_ms":percentile(0.95),"p99_ms":percentile(0.99),"max_ms":samples.last(),"mean_ms":samples.iter().sum::<f64>()/samples.len() as f64})
}
fn emit(value: Value) {
    println!("{value}");
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    if std::env::var("GL_BENCH_DISPOSABLE").as_deref() != Ok("1") {
        return Err("set GL_BENCH_DISPOSABLE=1 for an isolated test server".into());
    }
    let file = std::env::args()
        .nth(1)
        .ok_or("usage: large_data FILE [REPORT]")?;
    let report_path = std::env::args().nth(2).ok_or("report path required")?;
    let token: Value = serde_json::from_reader(File::open(std::env::var("GL_TOKEN_FILE")?)?)?;
    let client = Client::connect_with_timeout(
        std::env::var("GL_ENDPOINT")?,
        token[0]["token"].as_str().ok_or("token")?,
        Duration::from_secs(600),
    )
    .await?;
    let started = Instant::now();
    let project = client
        .create_project(&format!("benchmark-{}", uuid::Uuid::new_v4()))
        .await?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<Vec<Value>>(2);
    let parse_file = file.clone();
    let parser = tokio::task::spawn_blocking(move || -> Result<u64, Failure> {
        let mut batch = Vec::new();
        let mut bytes = 0;
        let mut count = 0;
        input::read(BufReader::new(File::open(parse_file)?), &mut |value| {
            count += 1;
            let value = input::feature(value, count)?;
            bytes += serde_json::to_vec(&value).map_err(|e| e.to_string())?.len();
            batch.push(value);
            if batch.len() >= 100 || bytes >= 1024 * 1024 {
                sender
                    .blocking_send(std::mem::take(&mut batch))
                    .map_err(|e| e.to_string())?;
                bytes = 0;
            }
            Ok(())
        })?;
        if !batch.is_empty() {
            sender.blocking_send(batch)?;
        }
        Ok(count)
    });
    let first = receiver.recv().await.ok_or("no input features")?;
    let sample = first.first().ok_or("empty batch")?.clone();
    let (family, dimension) = input::shape(&sample)?;
    let dataset = client
        .create_dataset_with_dimension(&project.id, "source", family, dimension)
        .await?;
    let mut workspace = client.create_workspace(&project.id).await?;
    let mut report = json!({"file":file,"bytes":std::fs::metadata(&file)?.len(),"server":client.info().await?,"project":project.id,"dataset":dataset.id,"geometry_type":family,"dimension":dimension});
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    let mut expected = sha2::Sha256::new();
    let mut saves = Vec::new();
    let mut count = 0u64;
    let mut warnings = 0usize;
    let mut next = Some(first);
    while let Some(batch) = next {
        let mut edits = Vec::with_capacity(batch.len());
        for feature in batch {
            input::digest(&mut expected, &feature)?;
            edits.push(Edit {
                dataset: dataset.id.clone(),
                feature_id: feature["id"].as_str().ok_or("id")?.into(),
                feature: Some(feature),
            });
        }
        let clock = Instant::now();
        let saved = workspace.save_batch(&edits).await?;
        saves.push(clock.elapsed().as_secs_f64() * 1000.0);
        warnings += saved.warnings.len();
        count += edits.len() as u64;
        if saves.len() % 100 == 0 {
            emit(
                json!({"phase":"import","features":count,"seconds":started.elapsed().as_secs_f64()}),
            );
        }
        next = receiver.recv().await;
    }
    assert_eq!(parser.await??, count);
    report["features"] = json!(count);
    report["import_seconds"] = json!(started.elapsed().as_secs_f64());
    report["save"] = summary(saves);
    report["warning_count"] = json!(warnings);
    let clock = Instant::now();
    let receipt = workspace.publish("large-data baseline").await?;
    report["publish_seconds"] = json!(clock.elapsed().as_secs_f64());
    report["revision"] = json!(receipt.revision);
    assert_eq!(receipt.changes, count);
    assert_eq!(receipt, workspace.publish("large-data baseline").await?);
    emit(json!({"phase":"published","report":report}));
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    let clock = Instant::now();
    let mut actual = sha2::Sha256::new();
    let mut after = String::new();
    let mut read = 0u64;
    let mut pages = Vec::new();
    loop {
        let clock = Instant::now();
        let page = client
            .features(
                &project.id,
                &dataset.id,
                FeatureQuery {
                    revision: Some(receipt.revision),
                    after: after.clone(),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .await?;
        pages.push(clock.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(page.revision, receipt.revision);
        for feature in page.features {
            read += 1;
            assert_eq!(feature["id"], format!("{read:012}"));
            input::digest(&mut actual, &feature)?;
        }
        match page.next_after {
            Some(cursor) if cursor > after => after = cursor,
            None => break,
            _ => return Err("pagination did not advance".into()),
        }
    }
    assert_eq!(read, count);
    assert_eq!(
        actual.clone().finalize(),
        expected.clone().finalize(),
        "source/readback digest differs"
    );
    report["readback_seconds"] = json!(clock.elapsed().as_secs_f64());
    report["read_pages"] = summary(pages);
    report["digest"] = json!(
        actual
            .clone()
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    report["exact_readback"] = json!(true);
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;
    // Keep the dataset unchanged: edit + delete + undo on a separate draft and verify r1 visibility.
    let mut edit = client.create_workspace(&project.id).await?;
    let mut changed = sample.clone();
    changed["properties"]["benchmark"] = json!(true);
    edit.save(&dataset.id, changed).await?;
    edit.delete(&dataset.id, sample["id"].as_str().ok_or("id")?)
        .await?;
    let deleted = edit.publish("delete sample").await?;
    let mut undo = client.restore(&project.id, deleted.revision).await?;
    undo.publish("undo sample deletion").await?;
    let restored = client
        .execute(
            "features",
            json!({"project":project.id,"dataset":dataset.id,"feature_id":sample["id"]}),
        )
        .await?;
    let mut check = sha2::Sha256::new();
    input::digest(&mut check, &restored["features"][0])?;
    let mut original = sha2::Sha256::new();
    input::digest(&mut original, &sample)?;
    assert_eq!(check.clone().finalize(), original.clone().finalize());
    report["delete_undo_and_replay"] = json!(true);
    let mut loads = Vec::new();
    for concurrency in [1, 4, 20] {
        let clock = Instant::now();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..concurrency {
            let c = client.clone();
            let p = project.id.clone();
            let d = dataset.id.clone();
            let rev = receipt.revision;
            tasks.spawn(async move {
                let mut times: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
                let mut errors: BTreeMap<String, u64> = BTreeMap::new();
                for _ in 0..20 {
                    for (op, extra) in [
                        ("first_page", json!({"limit":20})),
                        (
                            "late_page",
                            json!({"limit":20,"after":format!("{:012}",count.saturating_sub(21))}),
                        ),
                        (
                            "empty_bbox",
                            json!({"limit":20,"bbox":[0,0,0.00001,0.00001]}),
                        ),
                        ("single", json!({"feature_id":"000000000001"})),
                    ] {
                        let mut query = json!({"project":p,"dataset":d,"revision":rev});
                        query
                            .as_object_mut()
                            .ok_or("query")?
                            .extend(extra.as_object().ok_or("extra")?.clone());
                        let start = Instant::now();
                        let result = c.execute("features", query).await;
                        times
                            .entry(op)
                            .or_default()
                            .push(start.elapsed().as_secs_f64() * 1000.0);
                        match result {
                            Ok(value) => {
                                if op != "single" {
                                    if value["features"].as_array().is_none() {
                                        return Err("invalid page response".into());
                                    }
                                } else if value["features"][0]["id"] != "000000000001" {
                                    return Err("invalid feature response".into());
                                }
                            }
                            Err(e) => *errors.entry(e.code).or_default() += 1,
                        }
                    }
                }
                Ok::<_, Failure>((times, errors))
            });
        }
        let mut times: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        let mut errors: BTreeMap<String, u64> = BTreeMap::new();
        while let Some(result) = tasks.join_next().await {
            let (t, e) = result??;
            for (k, v) in t {
                times.entry(k).or_default().extend(v);
            }
            for (k, v) in e {
                *errors.entry(k).or_default() += v;
            }
        }
        let metrics: BTreeMap<_, _> = times.into_iter().map(|(k, v)| (k, summary(v))).collect();
        let load = json!({"concurrency":concurrency,"seconds":clock.elapsed().as_secs_f64(),"metrics":metrics,"errors":errors});
        emit(json!({"phase":"queries","load":load}));
        loads.push(load);
    }
    report["query_loads"] = json!(loads);
    report["total_seconds"] = json!(started.elapsed().as_secs_f64());
    report["success"] = json!(
        loads
            .iter()
            .all(|v| v["errors"].as_object().is_some_and(|o| o.is_empty()))
    );
    std::fs::write(report_path, serde_json::to_vec_pretty(&report)?)?;
    emit(report.clone());
    if report["success"] != true {
        return Err("query errors observed; see report".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_rank_percentiles() {
        let s = summary((1..=100).map(f64::from).collect());
        assert_eq!(s["p95_ms"], 95.0);
        assert_eq!(s["p99_ms"], 99.0);
        assert_eq!(summary(vec![])["count"], 0);
    }
}
