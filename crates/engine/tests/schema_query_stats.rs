//! Opt-in database execution counts; the isolated server must preload pg_stat_statements.
#![allow(
    clippy::unwrap_used,
    reason = "isolated database fixture setup and assertions fail immediately"
)]
mod support;
use geoledger_engine::{Application, Storage};
use serde_json::{Value, json};

#[test]
#[ignore = "requires isolated geoledger_test with pg_stat_statements preloaded"]
fn postgis_save_schema_query_counts_are_request_scoped() {
    let dsn = std::env::var("GL_TEST_DATABASE_URL").unwrap();
    let (dsn, mut db) = support::connect_isolated(&dsn, "schema-query-stats").unwrap();
    support::reset_database(&mut db).unwrap();
    db.batch_execute("CREATE EXTENSION pg_stat_statements")
        .unwrap();
    let app = Application::new(Storage::Postgis(dsn));
    app.migrate().unwrap();
    let actor = uuid::Uuid::new_v4().to_string();
    let project = app
        .execute(
            &actor,
            "create_project",
            json!({"name":"schema query stats"}),
        )
        .unwrap()["project"]
        .clone();
    let call = |operation: &str, mut input: Value| {
        input["project"] = project.clone();
        app.execute(&actor, operation, input).unwrap()
    };
    let datasets = ["first", "second"].map(|name| {
        call(
            "create_dataset",
            json!({"name":name,"geometry_type":"point"}),
        )["dataset"]
            .clone()
    });
    for dataset_count in [1, 2] {
        for edit_count in [100, dataset_count] {
            let workspace = call("create_workspace", json!({}))["workspace"].clone();
            let edits = (0..edit_count)
                .map(|i| {
                    let key = format!("feature-{i:03}");
                    json!({"dataset":datasets[i % dataset_count],"feature_id":key,"feature":{"type":"Feature","id":key,"properties":{"label":format!("value-{i}")},"geometry":null}})
                })
                .collect::<Vec<_>>();
            db.query_one("SELECT pg_stat_statements_reset()", &[])
                .unwrap();
            let saved = call(
                "save",
                json!({"workspace":workspace,"expected_workspace_version":0,"edits":edits}),
            );
            assert_eq!(saved["changes"], edit_count);
            let stats = db.query(
                "SELECT query,calls,rows FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname=current_database()) AND (query LIKE 'SELECT dataset,definition FROM gl_workspace_schemas WHERE %' OR query LIKE 'SELECT definition FROM gl_schema_history WHERE %') ORDER BY query",
                &[],
            ).unwrap();
            assert_eq!(
                stats.len(),
                2,
                "both schema SELECTs must reach the database"
            );
            let read = |prefix: &str| {
                let row = stats
                    .iter()
                    .find(|row| row.get::<_, String>(0).starts_with(prefix))
                    .unwrap();
                (row.get::<_, i64>(1), row.get::<_, i64>(2))
            };
            let workspace = read("SELECT dataset,definition FROM gl_workspace_schemas");
            let history = read("SELECT definition FROM gl_schema_history");
            eprintln!(
                "pg_stat_statements database executions: datasets={dataset_count}, edits={edit_count}, workspace_calls={}, workspace_rows={}, history_calls={}, history_rows={}",
                workspace.0, workspace.1, history.0, history.1
            );
            assert_eq!(workspace, (1, 0));
            assert_eq!(history, (dataset_count as i64, 0));
        }
    }
}
