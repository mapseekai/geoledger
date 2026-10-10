//! Explicit capacity probe. Seeds through SQL so timings exclude public API ingestion.
use geoledger_engine::{Application, Storage};
use serde_json::json;
use std::{
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};
#[test]
#[ignore = "explicit million-feature / 20-user capacity probe; needs several GB of free space"]
fn million_features_twenty_distinct_writers() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("capacity.db");
    let dsn = std::env::var("GL_TEST_DATABASE_URL").ok();
    let postgres = std::env::var("GL_CONFORMANCE_BACKEND").as_deref() == Ok("postgis");
    let storage = if postgres {
        let dsn = dsn.clone().ok_or("GL_TEST_DATABASE_URL")?;
        let mut c = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
        let name: String = c.query_one("SELECT current_database()", &[])?.get(0);
        assert_eq!(name, "geoledger_test");
        Storage::Postgis(dsn)
    } else {
        Storage::Sqlite(path.clone())
    };
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
    if postgres {
        let mut c = ::postgres::Client::connect(dsn.as_deref().ok_or("dsn")?, ::postgres::NoTls)?;
        let mut tx = c.transaction()?;
        tx.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES($1,1,$2,'capacity-0','SQL fixture')",&[&p,&w])?;
        tx.execute("INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,geom) SELECT $1,$2,lpad(i::text,7,'0'),1,$3,'{\"type\":\"Point\",\"coordinates\":['||(i%180)::text||',0]}',ST_SetSRID(ST_MakePoint(i%180,0),4326) FROM generate_series(1,1000000) AS s(i)",&[&p,&d,&props])?;
        tx.execute("UPDATE gl_projects SET head=1 WHERE id=$1", &[&p])?;
        tx.commit()?;
        c.batch_execute("ANALYZE gl_history;")?;
    } else {
        let c = rusqlite::Connection::open(&path)?;
        let extension =
            std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
        let mut native = geoledger_spatialite::load(c, std::path::Path::new(&extension))?;
        let mut c = native.write()?;
        let tx = c.transaction()?;
        tx.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES(?1,1,?2,'capacity-0','SQL fixture')",[&p,&w])?;
        tx.execute("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000000) INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,geom) SELECT ?1,?2,printf('%07d',i),1,?3,'{\"type\":\"Point\",\"coordinates\":['||(i%180)||',0]}',MakePoint(i%180,0,4326) FROM n",[&p,&d,&props])?;
        tx.execute("UPDATE gl_projects SET head=1 WHERE id=?1", [&p])?;
        tx.commit()?;
        c.execute_batch("ANALYZE;")?;
    }
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
    eprintln!(
        "capacity backend={} users=20 cycles=100 seconds={:.3} p50={:.3} p95={:.3} max={:.3}",
        app.backend(),
        started.elapsed().as_secs_f64(),
        times[49].as_secs_f64(),
        times[94].as_secs_f64(),
        times[99].as_secs_f64()
    );
    Ok(())
}
