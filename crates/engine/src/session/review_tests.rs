//! Measured work bounds for the review fixes, not wall-clock throughput claims.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn sqlite_count_metadata_and_stage_work_stays_bounded() -> TestResult {
    let dir = tempfile::tempdir()?;
    let storage = Storage::Sqlite(dir.path().join("bounded.db"));
    let pool = Arc::new(postgres::Pool::default());
    Client::open(&storage, &pool, Duration::from_secs(30))?.migrate()?;
    let mut t = Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
    t.batch_execute("INSERT INTO gl_projects(id,name) VALUES('p','p'); INSERT INTO gl_project_members(project,subject,role) VALUES('p','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('p','d','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('p','w','alice',0);")?;
    let steps = Arc::new(AtomicUsize::new(0));
    let observed = steps.clone();
    if let Backend::Sqlite(c) = &t.0.backend {
        let c = c.read().map_err(sqlite_error)?;
        c.progress_handler(
            1,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        )?;
    }
    for target in [1000_i64, 10000, 100000] {
        let have: i64 = t.count_deltas("p", "w")?.get(0usize)?;
        t.execute("WITH RECURSIVE ids(n) AS (SELECT $1 UNION ALL SELECT n+1 FROM ids WHERE n<$2) INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties) SELECT 'p','w','d',printf('%08d',n),'{}' FROM ids", &[&(have+1),&target])?;
        steps.store(0, Ordering::Relaxed);
        assert_eq!(t.count_deltas("p", "w")?.get::<_, i64>(0usize)?, target);
        let count_steps = steps.swap(0, Ordering::Relaxed);
        assert!(
            count_steps < 100,
            "count scanned {target} drafts: {count_steps}"
        );
        eprintln!("derived_count drafts={target} vm_steps={count_steps}");
    }
    t.dataset_exists("p", "d")?.ok_or("dataset")?;
    steps.store(0, Ordering::Relaxed);
    for _ in 0..1000 {
        t.dataset_exists("p", "d")?.ok_or("dataset")?;
    }
    assert_eq!(
        steps.load(Ordering::Relaxed),
        0,
        "metadata cache re-queried database"
    );
    t.begin_merge()?;
    for n in 0..100 {
        t.stage_merge("d", &n.to_string(), &None, &Some("{}".into()))?;
    }
    let staged: i64 = t
        .query_one("SELECT count(*) FROM gl_merge_stage", &[])?
        .get(0usize)?;
    assert_eq!(
        staged, 96,
        "three full 32-row batches must already be flushed"
    );
    assert_eq!(t.1.stage.len(), 4);
    t.flush_stage()?;
    assert_eq!(
        t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
            .get::<_, i64>(0usize)?,
        100
    );
    eprintln!(
        "metadata_calls=1000 additional_vm_steps=0; staged_rows=100 full_batches=3 final_batch=4"
    );
    Ok(())
}

#[test]
fn sqlite_conflict_cache_reuses_analysis_and_invalidates_on_version() -> TestResult {
    use serde_json::{Value, json};
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("cache.db");
    let app = crate::Application::new(Storage::Sqlite(path.clone()));
    app.migrate()?;
    let p = uuid::Uuid::new_v4().to_string();
    let d = uuid::Uuid::new_v4().to_string();
    let w = uuid::Uuid::new_v4().to_string();
    let db = rusqlite::Connection::open(&path)?;
    let extension =
        std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
    let db = geoledger_spatialite::load(db, std::path::Path::new(&extension))?;
    db.read()?.execute_batch(&format!("PRAGMA foreign_keys=ON; BEGIN; INSERT INTO gl_projects(id,name,head) VALUES('{p}','cache',2); INSERT INTO gl_project_members(project,subject,role) VALUES('{p}','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('{p}','{d}','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('{p}','{w}','alice',1); INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES('{p}',1,'{w}','alice','base'),('{p}',2,'{w}','alice','current'); WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<128) INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties) SELECT '{p}','{d}',printf('%08d',n),1,2,'{{\"a\":0}}' FROM ids; INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties) SELECT project,dataset,feature_id,2,'{{\"a\":1}}' FROM gl_history WHERE project='{p}'; INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties) SELECT project,'{w}',dataset,feature_id,'{{\"a\":2}}' FROM gl_history WHERE project='{p}' AND valid_from=1; COMMIT;"))?;
    let query = |after: &str| {
        app.execute(
            "alice",
            "conflicts",
            json!({"project":p,"workspace":w,"after":after,"limit":10}),
        )
    };
    assert_eq!(query("")?["total"], 128);
    db.read()?.execute_batch("CREATE TRIGGER reject_cache_rebuild BEFORE DELETE ON gl_conflict_cache BEGIN SELECT RAISE(ABORT,'unchanged cache was rebuilt'); END;")?;
    let page = query(&format!("{d}/00000100"))?;
    assert_eq!(page["conflicts"].as_array().ok_or("conflicts")?.len(), 10);
    assert_eq!(page["total"], 128);
    assert!(
        query(&format!("{d}/00000128"))?["conflicts"]
            .as_array()
            .ok_or("conflicts")?
            .is_empty()
    );
    db.read()?
        .execute_batch("DROP TRIGGER reject_cache_rebuild")?;
    app.execute("alice","save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"00000001","feature":{"type":"Feature","id":"00000001","properties":{"a":1},"geometry":Value::Null}}]}))?;
    assert_eq!(query("")?["total"], 127);
    let size: i64 = db
        .read()?
        .query_row("SELECT count(*) FROM gl_conflict_cache", [], |row| {
            row.get(0)
        })?;
    assert_eq!(size, 1, "old cache generations accumulated");
    eprintln!(
        "conflicts=128 cached_late_page=10 cached_empty_page=0 recomputations=0; changed_version_conflicts=127 cache_generations=1"
    );
    Ok(())
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_hot_spatial_queries_and_current_history_use_bounded_plans() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
    let name: String = admin.query_one("SELECT current_database()", &[])?.get(0);
    if name != "geoledger_test" {
        return Err("requires isolated geoledger_test".into());
    }
    let storage = Storage::Postgis(dsn);
    let pool = Arc::new(postgres::Pool::default());
    let timeout = Duration::from_secs(30);
    Client::open(&storage, &pool, timeout)?.migrate()?;
    let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
    let p = uuid::Uuid::new_v4().to_string();
    let d = uuid::Uuid::new_v4().to_string();
    let w = uuid::Uuid::new_v4().to_string();
    t.batch_execute(&format!("INSERT INTO gl_projects(id,name,head) VALUES('{p}','history plans',1000); INSERT INTO gl_project_members(project,subject,role) VALUES('{p}','alice','owner'); INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES('{p}','{d}','points','point'); INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES('{p}','{w}','alice',0); INSERT INTO gl_commits(project,revision,workspace,subject,message) SELECT '{p}',n,'{w}','alice','version' FROM generate_series(1,1000)n; INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties,geometry_json,geom) SELECT '{p}','{d}',lpad(k::text,8,'0'),v,CASE WHEN v<1000 THEN v+1 END,'{{\"v\":'||v::text||'}}','{{\"type\":\"Point\",\"coordinates\":[0,0]}}',ST_SetSRID(ST_MakePoint(0,0),0) FROM generate_series(1,64)k CROSS JOIN generate_series(1,1000)v; ANALYZE gl_history;"))?;
    t.project_head(&p, false)?.ok_or("project")?;
    let query = crate::repository::FeatureQuery {
        project: p.clone(),
        dataset: d.clone(),
        revision: 1000,
        workspace: None,
        after: String::new(),
        feature_id: None,
        bbox: Some([-180., -90., 180., 90.]),
        limit: 32,
    };
    for _ in 0..10 {
        assert_eq!(t.feature_page(&query)?.len(), 32);
    }
    let statements=t.query("SELECT name,generic_plans,custom_plans FROM pg_prepared_statements WHERE statement LIKE 'WITH base AS MATERIALIZED%'",&[])?;
    let statement = statements.first().ok_or("cached spatial statement")?;
    assert_eq!(
        statement.get::<_, i64>(1usize)?,
        0,
        "generic bbox plan was reused"
    );
    assert!(statement.get::<_, i64>(2usize)? >= 10);
    let name: String = statement.get(0usize)?;
    let rows=t.query(&format!("EXPLAIN (ANALYZE, BUFFERS, COSTS OFF) EXECUTE \"{}\" ('{p}','{d}',1000,NULL,'',NULL,true,-180,-90,180,90,32)",name.replace('"',"\"\"")),&[])?;
    let plan = rows
        .iter()
        .map(|row| row.get::<_, String>(0usize))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    assert!(
        plan.contains("history_live"),
        "current-head query missed partial index: {plan}"
    );
    assert!(
        !plan.contains("Rows Removed by Filter: 31968"),
        "old versions still scanned: {plan}"
    );
    eprintln!(
        "hot_bbox_executions=10 generic_plans=0 current_index=history_live logical_features=64 versions_each=1000 page_rows=32\n{plan}"
    );
    let historic = crate::repository::FeatureQuery {
        revision: 1,
        bbox: None,
        ..query
    };
    let rows = t.feature_page(&historic)?;
    assert_eq!(rows.len(), 32);
    let properties: serde_json::Value = serde_json::from_str(&rows[0].get::<_, String>(1usize)?)?;
    assert_eq!(
        properties["v"], 1,
        "latest optimization broke historical reads"
    );
    Ok(())
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_hot_spatial_queries_merge_stage_lookup_is_indexed() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
    let database: String = admin.query_one("SELECT current_database()", &[])?.get(0);
    if database != "geoledger_test" {
        return Err("requires isolated geoledger_test".into());
    }
    let storage = Storage::Postgis(dsn);
    let pool = Arc::new(postgres::Pool::default());
    let mut t = Client::open(&storage, &pool, Duration::from_secs(30))?.transaction(false)?;
    t.begin_merge()?;
    for n in 0..500 {
        t.stage_merge("d", &n.to_string(), &None, &Some(n.to_string()))?;
    }
    t.flush_stage()?;
    // Fresh tracking tables can estimate one key; verification must not scan the stage per key.
    t.batch_execute("CREATE TEMP TABLE tracked_keys(schema_name text,table_name text,feature_id text,PRIMARY KEY(schema_name,table_name,feature_id)); INSERT INTO tracked_keys SELECT 'business','source',feature_id FROM gl_merge_stage")?;
    let rows = t.query("EXPLAIN (ANALYZE, COSTS OFF) SELECT EXISTS(SELECT 1 FROM tracked_keys k LEFT JOIN gl_merge_stage s ON s.dataset='d' AND s.feature_id=k.feature_id WHERE k.schema_name='business' AND k.table_name='source' AND s.after_value IS DISTINCT FROM k.feature_id)", &[])?;
    let plan = rows
        .iter()
        .map(|r| r.get::<_, String>(0usize))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    assert!(
        plan.contains("gl_merge_stage_pkey"),
        "stage lookup lost index: {plan}"
    );
    assert!(
        !plan.contains("Rows Removed by Join Filter: 249500"),
        "quadratic stage scan: {plan}"
    );
    eprintln!("{plan}");
    Ok(())
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_hot_spatial_queries_pipeline_preserves_statement_budgets() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
    let name: String = admin.query_one("SELECT current_database()", &[])?.get(0);
    assert_eq!(name, "geoledger_test");
    let pool = Arc::new(postgres::Pool::new(crate::StorageOptions {
        statement_timeout: Duration::from_millis(80),
        lock_timeout: Duration::from_millis(40),
        ..crate::StorageOptions::default()
    }));
    for batch in [false, true] {
        let mut client = pool.connect(&dsn, Duration::from_secs(5))?;
        client.batch_execute("SET statement_timeout=0; SET lock_timeout=0")?;
        let row = client.query(
            "SELECT current_setting('statement_timeout'),current_setting('lock_timeout')",
            &[],
        )?;
        assert_eq!(row[0].get::<_, String>(0), "80ms");
        assert_eq!(row[0].get::<_, String>(1), "40ms");
        client.batch_execute("SET statement_timeout=0")?;
        let result = if batch {
            client.batch_execute("SELECT pg_sleep(1)")
        } else {
            client.query("SELECT pg_sleep(1)", &[]).map(|_| ())
        };
        assert_eq!(result.err().ok_or("statement must time out")?.status, 504);
    }
    let pool = Arc::new(postgres::Pool::default());
    let mut client = pool.connect(&dsn, Duration::from_secs(2))?;
    let before: i64 = client.query(
        "SELECT setting::bigint FROM pg_settings WHERE name='statement_timeout'",
        &[],
    )?[0]
        .get(0);
    std::thread::sleep(Duration::from_millis(30));
    let after: i64 = client.query(
        "SELECT setting::bigint FROM pg_settings WHERE name='statement_timeout'",
        &[],
    )?[0]
        .get(0);
    assert!(
        after < before && before <= 2000,
        "operation budget must shrink: {before} -> {after}"
    );
    Ok(())
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_hot_spatial_queries_merge_stage_reuses_relation_and_clears_transaction_rows()
-> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
    let database: String = admin.query_one("SELECT current_database()", &[])?.get(0);
    assert_eq!(database, "geoledger_test");
    let storage = Storage::Postgis(dsn);
    let pool = Arc::new(postgres::Pool::new(crate::StorageOptions {
        pool_size: 1,
        ..Default::default()
    }));
    let timeout = Duration::from_secs(30);
    let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
    t.begin_merge()?;
    let relation: String = t
        .query_one("SELECT 'pg_temp.gl_merge_stage'::regclass::oid::text", &[])?
        .get(0usize)?;
    t.stage_merge("d", "old", &None, &Some("{}".into()))?;
    t.flush_stage()?;
    t.commit()?;
    let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
    t.begin_merge()?;
    assert_eq!(
        t.query_one("SELECT 'pg_temp.gl_merge_stage'::regclass::oid::text", &[])?
            .get::<_, String>(0usize)?,
        relation
    );
    assert_eq!(
        t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
            .get::<_, i64>(0usize)?,
        0
    );
    t.stage_merge("d", "rollback", &None, &Some("{}".into()))?;
    t.flush_stage()?;
    drop(t);
    let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
    t.begin_merge()?;
    assert_eq!(
        t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
            .get::<_, i64>(0usize)?,
        0
    );
    t.stage_merge("d", "pending", &None, &Some("{}".into()))?;
    t.begin_merge()?;
    t.flush_stage()?;
    assert_eq!(
        t.query_one("SELECT count(*) FROM gl_merge_stage", &[])?
            .get::<_, i64>(0usize)?,
        0
    );
    t.commit()?;
    Ok(())
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_hot_spatial_queries_publication_looks_up_changed_history() -> TestResult {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let mut admin = ::postgres::Client::connect(&dsn, ::postgres::NoTls)?;
    let database: String = admin.query_one("SELECT current_database()", &[])?.get(0);
    assert_eq!(database, "geoledger_test");
    let storage = Storage::Postgis(dsn);
    let pool = Arc::new(postgres::Pool::default());
    let timeout = Duration::from_secs(30);
    Client::open(&storage, &pool, timeout)?.migrate()?;
    let mut t = Client::open(&storage, &pool, timeout)?.transaction(false)?;
    let project = uuid::Uuid::new_v4().to_string();
    t.execute(
        "INSERT INTO gl_projects(id,name) VALUES($1,'plan')",
        &[&project],
    )?;
    t.execute(
        "INSERT INTO gl_project_members(project,subject,role) VALUES($1,'alice','owner')",
        &[&project],
    )?;
    t.execute(
        "INSERT INTO gl_datasets(project,id,name,geometry_type) VALUES($1,'d','points','point')",
        &[&project],
    )?;
    t.execute(
        "INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES($1,'w','alice',0)",
        &[&project],
    )?;
    t.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES($1,1,'w','alice','seed'),($1,2,'w','alice','edit')", &[&project])?;
    t.execute("INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties) SELECT $1,'d',lpad(n::text,8,'0'),1,'{}' FROM generate_series(1,10000) n", &[&project])?;
    t.batch_execute("ANALYZE gl_history")?;
    t.begin_merge()?;
    t.stage_merge("d", "00000001", &Some("{}".into()), &None)?;
    t.close_history(&project, 2)?;
    let sql: String = t.query_one("SELECT statement FROM pg_prepared_statements WHERE statement LIKE 'UPDATE gl_history%SET valid_to%'", &[])?.get(0usize)?;
    let plan = t
        .query(
            &format!("EXPLAIN (ANALYZE, COSTS OFF) {sql}"),
            &[&project, &2_i64],
        )?
        .iter()
        .map(|r| r.get::<_, String>(0usize))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    assert!(
        plan.contains("Index Scan using history_live"),
        "publication lost live-key lookup: {plan}"
    );
    assert!(
        !plan.contains("Seq Scan on gl_history"),
        "small publication scanned history: {plan}"
    );
    assert_eq!(
        t.query_one(
            "SELECT count(*) FROM gl_history WHERE project=$1 AND valid_to=2",
            &[&project]
        )?
        .get::<_, i64>(0usize)?,
        1
    );
    eprintln!("{plan}");
    Ok(())
}
