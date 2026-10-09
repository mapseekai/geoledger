//! Explicit real-file test, separate from the default conformance fixtures.
use geoledger_engine::{Application, Storage};
use serde_json::{Value, json};
use std::time::Instant;

// JSON has a single number type: the existing codec may emit 112 for 112.0.
// Remove only a zero fractional suffix, never round through f64 (properties
// can contain exact integers beyond binary64's range).
fn canonical_numbers(value: &Value) -> Value {
    match value {
        Value::Number(n) => {
            let text = n.to_string();
            if text.contains('.') && !text.contains(['e', 'E']) {
                let trimmed = text.trim_end_matches('0').trim_end_matches('.');
                if let Ok(number) = serde_json::from_str(trimmed) {
                    return number;
                }
            }
            value.clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical_numbers).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), canonical_numbers(v)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[test]
#[ignore = "opt-in real GeoJSON: set GL_LARGE_GEOJSON; creates a disposable database"]
fn original_geometries_survive_native_storage() -> Result<(), Box<dyn std::error::Error>> {
    let input: Value = serde_json::from_reader(std::io::BufReader::new(std::fs::File::open(
        std::env::var("GL_LARGE_GEOJSON")?,
    )?))?;
    let features = input["features"]
        .as_array()
        .ok_or("FeatureCollection required")?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("native.db");
    let app = Application::new(Storage::Sqlite(path.clone()))
        .with_timeout(std::time::Duration::from_secs(600));
    app.migrate()?;
    let p =
        app.execute("native", "create_project", json!({"name":"native test"}))?["project"].clone();
    let d = app.execute(
        "native",
        "create_dataset",
        json!({"project":p,"name":"polygons","geometry_type":"polygon"}),
    )?["dataset"]
        .clone();
    let w = app.execute("native", "create_workspace", json!({"project":p}))?["workspace"].clone();
    let started = Instant::now();
    let mut version = 0;
    for (batch, values) in features.chunks(100).enumerate() {
        let edits: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let id = format!("{:08}", batch * 100 + index);
                let mut feature = value.clone();
                feature["id"] = json!(id);
                json!({"dataset":d,"feature_id":id,"feature":feature})
            })
            .collect();
        app.execute(
            "native",
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":version,"edits":edits}),
        )?;
        version += 1;
    }
    eprintln!(
        "native save {} features: {:?}",
        features.len(),
        started.elapsed()
    );
    let started = Instant::now();
    app.execute("native", "publish", json!({"project":p,"workspace":w,"expected_workspace_version":version,"request_id":uuid::Uuid::new_v4(),"message":"native fixture"}))?;
    eprintln!("native publish: {:?}", started.elapsed());
    let started = Instant::now();
    let summary = app.execute(
        "native",
        "commit_summary",
        json!({"project":p,"revision":1}),
    )?;
    assert_eq!(summary["total"]["added"], json!(features.len()));
    eprintln!("native summary: {:?}", started.elapsed());
    let started = Instant::now();
    let mut after = String::new();
    let mut count = 0;
    loop {
        let page = app.execute("native", "features", json!({"project":p,"dataset":d,"revision":1,"after":after,"limit":100,"bbox":[-180,-90,180,90]}))?;
        let rows = page["features"].as_array().ok_or("features")?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            assert_eq!(
                canonical_numbers(&row["geometry"]),
                canonical_numbers(&features[count]["geometry"]),
                "geometry at feature {count}"
            );
            assert_eq!(
                canonical_numbers(&row["properties"]),
                canonical_numbers(&features[count]["properties"]),
                "properties at feature {count}"
            );
            after = row["id"].as_str().ok_or("id")?.to_owned();
            count += 1;
        }
    }
    assert_eq!(count, features.len());
    eprintln!("native bbox/full read {count}: {:?}", started.elapsed());
    let c = rusqlite::Connection::open(path)?;
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM gl_history WHERE typeof(geom)='blob'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        features.len() as i64
    );
    Ok(())
}
