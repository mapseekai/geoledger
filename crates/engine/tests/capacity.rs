//! Explicit capacity probe. Seeds through SQL so timings exclude public API ingestion.
mod support;
use geoledger_engine::{Application, Storage};
use serde_json::json;
use std::{
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};
struct ResetPostgis(Option<String>);
impl Drop for ResetPostgis {
    fn drop(&mut self) {
        if let Some(dsn) = &self.0
            && let Ok(mut db) = ::postgres::Client::connect(dsn, ::postgres::NoTls)
        {
            let _ = support::reset_database(&mut db);
        }
    }
}
#[test]
#[ignore = "explicit million-feature / 20-user capacity probe; needs several GB of free space"]
fn million_features_twenty_distinct_writers() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("capacity.db");
    let dsn = std::env::var("GL_TEST_DATABASE_URL").ok();
    let postgres = std::env::var("GL_CONFORMANCE_BACKEND").as_deref() == Ok("postgis");
    let storage = if postgres {
        let dsn = dsn.clone().ok_or("GL_TEST_DATABASE_URL")?;
        let (dsn, mut c) = support::connect_isolated(&dsn, "capacity")?;
        support::reset_database(&mut c)?;
        Storage::Postgis(dsn)
    } else {
        Storage::Sqlite(path.clone())
    };
    let mut cleanup = ResetPostgis(postgres.then(|| dsn.clone()).flatten());
    let app = Application::new(storage);
    app.migrate()?;
    let p = app.execute("capacity-0", "create_project", json!({"name":"capacity"}))?["project"]
        .as_str()
        .ok_or("project")?
        .to_owned();
    let d = app.execute(
        "capacity-0",
        "create_dataset",
        json!({"geometry_type":"point","project":p,"name":"points"}),
    )?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    let w = app.execute("capacity-0", "create_workspace", json!({"project":p}))?["workspace"]
        .as_str()
        .ok_or("workspace")?
        .to_owned();
    for i in 1..20 {
        app.execute(
            "capacity-0",
            "set_member",
            json!({"project":p,"subject":format!("capacity-{i}"),"role":"editor"}),
        )?;
    }
    let seed = Instant::now();
    let props = json!({"payload":"x".repeat(512)}).to_string();
    let table = if postgres {
        let mut c = ::postgres::Client::connect(dsn.as_deref().ok_or("dsn")?, ::postgres::NoTls)?;
        let mut tx = c.transaction()?;
        tx.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES($1,1,$2,'capacity-0','SQL fixture')",&[&p,&w])?;
        let table: String = tx
            .query_one(
                "SELECT storage_table FROM gl_datasets WHERE project=$1 AND id=$2",
                &[&p, &d],
            )?
            .get(0);
        // Seed the sole current image directly; history starts empty.
        tx.query_one(
            "SELECT set_config('geoledger.publication_table',($1::text::regclass::oid)::text,true)",
            &[&table],
        )?;
        tx.execute(&format!("INSERT INTO {table}(feature_id,valid_from,properties,geometry_json,geom) SELECT lpad(i::text,7,'0'),1,$1,'{{\"type\":\"Point\",\"coordinates\":['||(i%180)::text||',0]}}',ST_SetSRID(ST_MakePoint(i%180,0),0) FROM generate_series(1,1000000) AS s(i)"), &[&props])?;
        assert_eq!(
            tx.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
                .get::<_, i64>(0),
            1_000_000
        );
        tx.execute("UPDATE gl_projects SET head=1 WHERE id=$1", &[&p])?;
        tx.commit()?;
        c.batch_execute(&format!("ANALYZE {table};"))?;
        table
    } else {
        let c = rusqlite::Connection::open(&path)?;
        let native = geoledger_spatialite::load_configured(c)?;
        let c = native.read()?;
        let tx = c.unchecked_transaction()?;
        tx.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES(?1,1,?2,'capacity-0','SQL fixture')",[&p,&w])?;
        let table: String = tx.query_row(
            "SELECT storage_table FROM gl_datasets WHERE project=?1 AND id=?2",
            [&p, &d],
            |r| r.get(0),
        )?;
        tx.execute_batch(&format!(
            "DROP TRIGGER {table}_insert; DROP TRIGGER {table}_update; DROP TRIGGER {table}_delete"
        ))?;
        tx.execute(&format!("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000000) INSERT INTO {table}(feature_id,valid_from,properties,geometry_json,geom) SELECT printf('%07d',i),1,?1,'{{\"type\":\"Point\",\"coordinates\":['||(i%180)||',0]}}',MakePoint(i%180,0,0) FROM n"),[&props])?;
        tx.execute_batch(&format!("CREATE TRIGGER {table}_insert BEFORE INSERT ON {table} WHEN NOT gl_dataset_write_allowed() BEGIN SELECT RAISE(ABORT,'write this dataset through GeoLedger'); END; CREATE TRIGGER {table}_update BEFORE UPDATE ON {table} WHEN OLD.feature_id<>NEW.feature_id OR NOT gl_dataset_write_allowed() BEGIN SELECT RAISE(ABORT,'write this dataset through GeoLedger'); END; CREATE TRIGGER {table}_delete BEFORE DELETE ON {table} WHEN NOT gl_dataset_write_allowed() BEGIN SELECT RAISE(ABORT,'write this dataset through GeoLedger'); END"))?;
        assert_eq!(
            tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))?,
            1_000_000
        );
        tx.execute("UPDATE gl_projects SET head=1 WHERE id=?1", [&p])?;
        tx.commit()?;
        c.execute_batch("ANALYZE;")?;
        table
    };
    eprintln!("capacity backend={} rows=1000000 GeoJSON_estimate_bytes={} seed_secs={:.3}",app.backend(),serde_json::to_string(&json!({"type":"Feature","id":"0000001","properties":{"payload":"x".repeat(512)},"geometry":{"type":"Point","coordinates":[100,0]}}))?.len()*1_000_000,seed.elapsed().as_secs_f64());
    let barrier = Arc::new(Barrier::new(20));
    let started = Instant::now();
    let mut threads = Vec::new();
    for user in 0..20 {
        let app = app.clone();
        let p = p.clone();
        let d = d.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move||->Result<Vec<Duration>,geoledger_engine::Error>{
   let subject=format!("capacity-{user}");let mut times=Vec::new();barrier.wait();
   for round in 0..5 {
    let start=Instant::now();
    let page=app.execute(&subject,"features",json!({"project":p,"dataset":d,"revision":1,"after":"0900000","limit":100}))?;
    assert_eq!(page["features"].as_array().map(Vec::len),Some(100));
    let w=app.execute(&subject,"create_workspace",json!({"project":p}))?["workspace"].clone();let key=format!("{:07}",user*5+round+1);
    app.execute(&subject,"save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":key,"feature":{"type":"Feature","id":key,"properties":{"user":user,"round":round},"geometry":{"type":"Point","coordinates":[user,round]}}}]}))?;
    let req=json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"capacity"});
    let result=app.execute(&subject,"publish",req.clone())?;assert_eq!(app.execute(&subject,"publish",req)?,result);
    let read=app.execute(&subject,"features",json!({"project":p,"dataset":d,"feature_id":key}))?;assert_eq!(read["properties"]["user"],user);times.push(start.elapsed());
   }
   Ok(times)
  }));
    }
    let mut times = Vec::new();
    for t in threads {
        times.extend(t.join().map_err(|_| "capacity worker panicked")??);
    }
    times.sort();
    assert_eq!(
        app.execute("capacity-0", "get_project", json!({"project":p}))?["head"],
        101
    );
    let historic = app.execute(
        "capacity-0",
        "features",
        json!({"project":p,"dataset":d,"feature_id":"0000001","revision":1}),
    )?;
    assert_eq!(
        historic["properties"]["payload"].as_str().map(str::len),
        Some(512)
    );
    let tail = app.execute(
        "capacity-0",
        "features",
        json!({"project":p,"dataset":d,"after":"0999900","limit":100}),
    )?;
    assert_eq!(tail["features"].as_array().map(Vec::len), Some(100));
    assert_eq!(tail["features"][99]["id"], "1000000");
    assert_eq!(tail["next_after"], serde_json::Value::Null);
    let live_rows: i64 = if postgres {
        let mut c = ::postgres::Client::connect(dsn.as_deref().ok_or("dsn")?, ::postgres::NoTls)?;
        c.query_one(&format!("SELECT count(*) FROM {table}"), &[])?
            .get(0)
    } else {
        rusqlite::Connection::open(&path)?.query_row(
            &format!("SELECT count(*) FROM {table}"),
            [],
            |r| r.get(0),
        )?
    };
    assert_eq!(live_rows, 1_000_000);
    eprintln!(
        "capacity backend={} users=20 cycles=100 seconds={:.3} p50={:.3} p95={:.3} max={:.3}",
        app.backend(),
        started.elapsed().as_secs_f64(),
        times[49].as_secs_f64(),
        times[94].as_secs_f64(),
        times[99].as_secs_f64()
    );
    if postgres {
        let mut c = ::postgres::Client::connect(dsn.as_deref().ok_or("dsn")?, ::postgres::NoTls)?;
        support::reset_database(&mut c)?;
        cleanup.0 = None;
    }
    Ok(())
}
