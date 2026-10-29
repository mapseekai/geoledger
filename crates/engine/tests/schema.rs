#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixture setup and assertions fail immediately, including guarded environment admission"
)]
mod support;
use geoledger_engine::{Application, Storage};
use serde_json::{Value, json};
use uuid::Uuid;

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_schema_publication_is_atomic_historical_and_restorable() {
    let dsn = std::env::var("GL_TEST_DATABASE_URL").expect("dedicated test DSN required");
    let (dsn, mut db) = support::connect_isolated(&dsn, "schema-publication").unwrap();
    support::reset_database(&mut db).unwrap();
    let schema_name = format!("schema_test_{}", Uuid::new_v4().simple());
    let table = format!("{schema_name}.schema_points");
    db.batch_execute(&format!("CREATE SCHEMA {schema_name}; CREATE TABLE {table}(fid bigint PRIMARY KEY,title text,remove text,amount bigint,protected text NOT NULL,indexed text,geom geometry(PointZ,3857)); CREATE INDEX schema_points_indexed ON {table}(indexed); INSERT INTO {table} VALUES(1,'first','original',9007199254740993,'keep','index',ST_SetSRID(ST_MakePoint(10.123456789,20.987654321,30.75),3857)),(2,'second',NULL,12,'keep','other',NULL);")).unwrap();
    let app = Application::new(Storage::Postgis(dsn));
    app.migrate().unwrap();
    let password = Uuid::new_v4().to_string();
    app.bootstrap_admin("schema-fixture", &password).unwrap();
    let login = app
        .auth_operation(
            None,
            "login_password",
            json!({"username":"schema-fixture","password":password}),
        )
        .unwrap();
    let actor = login["user"]["id"].as_str().unwrap();
    let call = |operation: &str, input: Value| app.execute(actor, operation, input).unwrap();
    let project = call("create_project", json!({"name":schema_name}))["project"]
        .as_str()
        .unwrap()
        .to_owned();
    let dataset=call("create_dataset",json!({"project":project,"name":"points","postgis_table":{"schema":schema_name,"table":"schema_points","id_column":"fid","geometry_column":"geom"}}))["dataset"].as_str().unwrap().to_owned();
    let baseline = call(
        "features",
        json!({"project":project,"dataset":dataset,"revision":1}),
    );
    let oldschema = call(
        "schema",
        json!({"project":project,"dataset":dataset,"revision":1}),
    );
    let workspace = call("create_workspace", json!({"project":project}))["workspace"]
        .as_str()
        .unwrap()
        .to_owned();
    let intent =
        json!([{"dataset":dataset,"add":[{"name":"extra","type":"integer"}],"drop":["remove"]}]);
    call(
        "save",
        json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"schema_edits":intent}),
    );
    let draft = call(
        "features",
        json!({"project":project,"dataset":dataset,"workspace":workspace}),
    );
    assert!(draft["features"][0]["properties"].get("remove").is_none());
    assert_eq!(draft["features"][0]["properties"]["extra"], Value::Null);
    let trigger = format!("{schema_name}_receipt");
    db.batch_execute(&format!("CREATE FUNCTION {schema_name}.fail_schema_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'schema publication rollback fixture'; END $$; CREATE TRIGGER {trigger} BEFORE INSERT ON public.gl_idempotency FOR EACH ROW WHEN (NEW.project='{project}') EXECUTE FUNCTION {schema_name}.fail_schema_receipt();")).unwrap();
    let publication = json!({"project":project,"workspace":workspace,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"schema"});
    assert!(app.execute(actor, "publish", publication.clone()).is_err());
    let columns:Vec<String>=db.query("SELECT attname FROM pg_attribute WHERE attrelid=$1::text::regclass AND attnum>0 AND NOT attisdropped",&[&table]).unwrap().iter().map(|r|r.get(0)).collect();
    assert!(columns.contains(&"remove".into()));
    assert!(!columns.contains(&"extra".into()));
    assert_eq!(
        db.query_one(
            "SELECT count(*) FROM gl_commits WHERE project=$1",
            &[&project]
        )
        .unwrap()
        .get::<_, i64>(0),
        1
    );
    assert_eq!(
        call(
            "schema",
            json!({"project":project,"dataset":dataset,"workspace":workspace})
        )["workspace_version"],
        1
    );
    db.batch_execute(&format!("DROP TRIGGER {trigger} ON public.gl_idempotency; DROP FUNCTION {schema_name}.fail_schema_receipt();")).unwrap();
    let receipt = call("publish", publication.clone());
    assert_eq!(receipt, call("publish", publication));
    assert_eq!(receipt["revision"], 2);
    let head = call("features", json!({"project":project,"dataset":dataset}));
    assert_eq!(
        head["features"][0]["properties"]["amount"],
        json!(9007199254740993_i64)
    );
    assert_eq!(
        head["features"][0]["geometry"],
        baseline["features"][0]["geometry"]
    );
    assert_eq!(
        call(
            "features",
            json!({"project":project,"dataset":dataset,"revision":1})
        )["features"],
        baseline["features"]
    );
    assert_eq!(
        call(
            "schema",
            json!({"project":project,"dataset":dataset,"revision":1})
        ),
        oldschema
    );
    let bad = call("create_workspace", json!({"project":project}))["workspace"]
        .as_str()
        .unwrap()
        .to_owned();
    for name in ["fid", "geom", "protected"] {
        let rejected=app.execute(actor,"save",json!({"project":project,"workspace":bad,"expected_workspace_version":0,"schema_edits":[{"dataset":dataset,"drop":[name]}]})).unwrap_err();
        assert_eq!(rejected.status, 400);
    }
    call(
        "save",
        json!({"project":project,"workspace":bad,"expected_workspace_version":0,"schema_edits":[{"dataset":dataset,"drop":["indexed"]}]}),
    );
    let failure=app.execute(actor,"publish",json!({"project":project,"workspace":bad,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"reject dependency"})).unwrap_err();
    assert_eq!(failure.status, 400);
    assert_eq!(
        call(
            "schema",
            json!({"project":project,"dataset":dataset,"workspace":bad})
        )["workspace_version"],
        1
    );
    let restored = call("restore", json!({"project":project,"revision":2}));
    call(
        "publish",
        json!({"project":project,"workspace":restored["workspace"],"expected_workspace_version":0,"request_id":Uuid::new_v4(),"message":"restore schema"}),
    );
    assert_eq!(
        call("features", json!({"project":project,"dataset":dataset}))["features"],
        baseline["features"]
    );
    let columns:Vec<String>=db.query("SELECT attname FROM pg_attribute WHERE attrelid=$1::text::regclass AND attnum>0 AND NOT attisdropped",&[&table]).unwrap().iter().map(|r|r.get(0)).collect();
    assert!(columns.contains(&"remove".into()));
    assert!(!columns.contains(&"extra".into()));
    let dir = tempfile::tempdir().unwrap();
    let mut archive = Vec::new();
    app.export_data(&mut archive).unwrap();
    let imported = Application::new(Storage::Sqlite(dir.path().join("schema.db")));
    imported.migrate().unwrap();
    imported
        .import_data(&mut std::io::Cursor::new(archive))
        .unwrap();
    assert_eq!(
        imported
            .execute(
                actor,
                "features",
                json!({"project":project,"dataset":dataset,"revision":1})
            )
            .unwrap()["features"],
        baseline["features"]
    );
    assert_eq!(
        imported
            .execute(
                actor,
                "schema",
                json!({"project":project,"dataset":dataset,"revision":1})
            )
            .unwrap(),
        oldschema
    );
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_schema_saves_share_project_lock_and_preserve_workspace_cas() {
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::{Duration, Instant};
    let dsn = std::env::var("GL_TEST_DATABASE_URL").unwrap();
    let (dsn, mut db) = support::connect_isolated(&dsn, "schema-concurrency").unwrap();
    support::reset_database(&mut db).unwrap();
    let app = Application::new(Storage::Postgis(dsn));
    app.migrate().unwrap();
    let password = Uuid::new_v4().to_string();
    app.bootstrap_admin("schema-fixture", &password).unwrap();
    let actor = app
        .auth_operation(
            None,
            "login_password",
            json!({"username":"schema-fixture","password":password}),
        )
        .unwrap()["user"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let call = |op: &str, input: Value| app.execute(&actor, op, input).unwrap();
    let p = call(
        "create_project",
        json!({"name":format!("schema_lock_{}",Uuid::new_v4())}),
    )["project"]
        .as_str()
        .unwrap()
        .to_owned();
    let d = call(
        "create_dataset",
        json!({"project":p,"name":"empty points","geometry_type":"point"}),
    )["dataset"]
        .as_str()
        .unwrap()
        .to_owned();
    let workspaces = (0..2)
        .map(|_| {
            call("create_workspace", json!({"project":p}))["workspace"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    let mut holder = db.transaction().unwrap();
    holder
        .query_one("SELECT head FROM gl_projects WHERE id=$1 FOR SHARE", &[&p])
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let (send, recv) = mpsc::channel();
    let mut workers = Vec::new();
    for (index, w) in workspaces.iter().enumerate() {
        let (app, actor, p, d, w, barrier, send) = (
            app.clone(),
            actor.clone(),
            p.clone(),
            d.clone(),
            w.clone(),
            barrier.clone(),
            send.clone(),
        );
        workers.push(std::thread::spawn(move||{barrier.wait();let result=app.execute(&actor,"save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"add":[{"name":format!("field_{index}"),"type":"integer"}]}]}));send.send((index,result)).unwrap();}));
    }
    drop(send);
    barrier.wait();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut results = Vec::new();
    while results.len() < 2 {
        match recv.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(value) => results.push(value),
            Err(_) => break,
        }
    }
    let completed_while_shared = results.len();
    holder.rollback().unwrap();
    for worker in workers {
        worker.join().unwrap();
    }
    results.extend(recv.try_iter());
    eprintln!(
        "different workspaces: completed while third-party FOR SHARE held={completed_while_shared}; statuses={:?}",
        results
            .iter()
            .map(|(i, r)| (*i, r.as_ref().map(|_| 200).unwrap_or_else(|e| e.status)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        completed_while_shared, 2,
        "independent schema saves must not upgrade the project edit lock"
    );
    assert!(results.iter().all(|(_, r)| r.is_ok()));
    for (index, w) in workspaces.iter().enumerate() {
        let schema = call("schema", json!({"project":p,"dataset":d,"workspace":w}));
        assert_eq!(schema["workspace_version"], 1);
        assert_eq!(schema["fields"][0]["name"], format!("field_{index}"));
    }
    let shared = call("create_workspace", json!({"project":p}))["workspace"]
        .as_str()
        .unwrap()
        .to_owned();
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for index in 0..2 {
        let (app, actor, p, d, w, barrier) = (
            app.clone(),
            actor.clone(),
            p.clone(),
            d.clone(),
            shared.clone(),
            barrier.clone(),
        );
        workers.push(std::thread::spawn(move||{barrier.wait();app.execute(&actor,"save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"add":[{"name":format!("same_{index}"),"type":"integer"}]}]}))}));
    }
    barrier.wait();
    let mut statuses = workers
        .into_iter()
        .map(|w| w.join().unwrap().map(|_| 200).unwrap_or_else(|e| e.status))
        .collect::<Vec<_>>();
    statuses.sort();
    assert_eq!(statuses, vec![200, 409]);
    assert_eq!(
        call("get_workspace", json!({"project":p,"workspace":shared}))["version"],
        1
    );
    let mut holder = db.transaction().unwrap();
    holder
        .query_one("SELECT head FROM gl_projects WHERE id=$1 FOR SHARE", &[&p])
        .unwrap();
    let (send, recv) = mpsc::channel();
    let (app2, actor2, p2, w2) = (app.clone(), actor.clone(), p.clone(), workspaces[0].clone());
    let publisher = std::thread::spawn(move || {
        send.send(app2.execute(&actor2,"publish",json!({"project":p2,"workspace":w2,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"head exclusion"}))).unwrap();
    });
    let waited = recv.recv_timeout(Duration::from_millis(100));
    holder.rollback().unwrap();
    publisher.join().unwrap();
    assert!(
        waited.is_err(),
        "publisher must wait for the shared editing lock"
    );
    assert_eq!(recv.recv().unwrap().unwrap()["revision"], 1);
    assert_eq!(app.execute(&actor,"publish",json!({"project":p,"workspace":workspaces[1],"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"stale schema"})).unwrap_err().status,409);
    assert_eq!(
        app.execute(
            &Uuid::new_v4().to_string(),
            "save",
            json!({"project":p,"workspace":shared,"expected_workspace_version":1,"schema_edits":[]})
        )
        .unwrap_err()
        .status,
        404
    );
    assert_eq!(
        call("get_workspace", json!({"project":p,"workspace":shared}))["version"],
        1
    );
}

#[test]
#[ignore = "requires isolated geoledger_test PostGIS database"]
fn postgis_schema_purge_preserves_cross_dataset_drafts_history_and_receipts() {
    let dsn = std::env::var("GL_TEST_DATABASE_URL").unwrap();
    let (dsn, mut db) = support::connect_isolated(&dsn, "schema-purge").unwrap();
    support::reset_database(&mut db).unwrap();
    let app = Application::new(Storage::Postgis(dsn));
    app.migrate().unwrap();
    let password = Uuid::new_v4().to_string();
    app.bootstrap_admin("schema-fixture", &password).unwrap();
    let login = app
        .auth_operation(
            None,
            "login_password",
            json!({"username":"schema-fixture","password":password}),
        )
        .unwrap();
    let actor = login["user"]["id"].as_str().unwrap();
    for mode in ["mixed_draft", "schema_cas", "mixed_commit", "schema_commit"] {
        let name = format!("schema_purge_{}", Uuid::new_v4().simple());
        let project = app
            .execute(actor, "create_project", json!({"name":name}))
            .unwrap()["project"]
            .clone();
        let call = |op: &str, mut input: Value| {
            input["project"] = project.clone();
            app.execute(actor, op, input).unwrap()
        };
        let primary = call(
            "create_dataset",
            json!({"name":"primary","geometry_type":"point"}),
        )["dataset"]
            .clone();
        let survivor = call(
            "create_dataset",
            json!({"name":"survivor","geometry_type":"point"}),
        )["dataset"]
            .clone();
        let workspace = call("create_workspace", json!({}))["workspace"].clone();
        let mut schemas =
            vec![json!({"dataset":survivor,"add":[{"name":"shared","type":"string"}]})];
        if ["schema_cas", "schema_commit"].contains(&mode) {
            schemas.push(json!({"dataset":primary,"add":[{"name":"shared","type":"integer"}]}));
        }
        let edits = if ["mixed_draft", "mixed_commit"].contains(&mode) {
            vec![
                json!({"dataset":primary,"feature_id":"only","feature":{"type":"Feature","id":"only","properties":{"n":1},"geometry":null}}),
            ]
        } else {
            vec![]
        };
        call(
            "save",
            json!({"workspace":workspace,"expected_workspace_version":0,"schema_edits":schemas,"edits":edits}),
        );
        let request = json!({"workspace":workspace,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"schema purge fixture"});
        let published = ["mixed_commit", "schema_commit"].contains(&mode);
        let receipt = published.then(|| call("publish", request.clone()));
        call(
            "delete_dataset",
            json!({"dataset":primary,"confirm_name":"primary"}),
        );
        assert_eq!(
            db.query_one(
                "SELECT count(*) FROM gl_schema_history WHERE project=$1 AND dataset=$2",
                &[&project.as_str().unwrap(), &primary.as_str().unwrap()]
            )
            .unwrap()
            .get::<_, i64>(0),
            0
        );
        assert_eq!(
            db.query_one(
                "SELECT count(*) FROM gl_workspace_schemas WHERE project=$1 AND dataset=$2",
                &[&project.as_str().unwrap(), &primary.as_str().unwrap()]
            )
            .unwrap()
            .get::<_, i64>(0),
            0
        );
        assert_eq!(
            call("get_workspace", json!({"workspace":workspace}))["version"],
            if published { 3 } else { 2 }
        );
        let schema = call("schema", json!({"dataset":survivor,"workspace":workspace}));
        assert_eq!(schema["fields"][0]["name"], "shared");
        assert_eq!(schema["fields"][0]["type"], "text");
        assert!(
            db.execute(
                "DELETE FROM gl_schema_history WHERE project=$1",
                &[&project.as_str().unwrap()]
            )
            .is_err()
        );
        if let Some(receipt) = receipt {
            assert_eq!(
                call("history", json!({}))["commits"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            let commit = call("commit", json!({"revision":1}));
            assert_eq!(commit["changes"], json!([]));
            assert_eq!(commit["schema_changes"].as_array().unwrap().len(), 1);
            assert_eq!(commit["schema_changes"][0]["dataset"], survivor);
            assert_eq!(call("publish", request.clone()), receipt);
            let inverse = call("restore", json!({"revision":1}))["workspace"].clone();
            call(
                "publish",
                json!({"workspace":inverse,"expected_workspace_version":0,"request_id":Uuid::new_v4(),"message":"restore survivor"}),
            );
            assert_eq!(
                call("schema", json!({"dataset":survivor}))["fields"],
                json!([])
            );
            assert_eq!(
                call("schema", json!({"dataset":survivor,"revision":1}))["fields"],
                schema["fields"]
            );
            assert_eq!(call("publish", request), receipt);
        } else {
            assert_eq!(
                call("diff", json!({"workspace":workspace}))["schema_changes"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            let stale = app.execute(actor,"save",json!({"project":project,"workspace":workspace,"expected_workspace_version":1,"schema_edits":[]})).unwrap_err();
            assert_eq!(stale.status, 409);
            call(
                "publish",
                json!({"workspace":workspace,"expected_workspace_version":2,"request_id":Uuid::new_v4(),"message":"retained draft"}),
            );
        }
        let draft = call("create_workspace", json!({}))["workspace"].clone();
        call(
            "save",
            json!({"workspace":draft,"expected_workspace_version":0,"schema_edits":[{"dataset":survivor,"add":[{"name":"pending","type":"boolean"}]}]}),
        );
        call("delete_project", json!({"confirm_name":name}));
        for table in [
            "gl_workspace_schemas",
            "gl_schema_history",
            "gl_workspace_changes",
            "gl_history",
            "gl_commit_changes",
            "gl_idempotency",
            "gl_commits",
            "gl_workspaces",
            "gl_datasets",
            "gl_purge",
        ] {
            assert_eq!(
                db.query_one(
                    &format!("SELECT count(*) FROM {table} WHERE project=$1"),
                    &[&project.as_str().unwrap()]
                )
                .unwrap()
                .get::<_, i64>(0),
                0,
                "{mode} {table}"
            );
        }
        assert!(
            db.query_one(
                "SELECT count(*) FROM gl_audit_events WHERE project=$1",
                &[&project.as_str().unwrap()]
            )
            .unwrap()
            .get::<_, i64>(0)
                > 0
        );
        assert!(
            db.execute(
                "DELETE FROM gl_audit_events WHERE project=$1",
                &[&project.as_str().unwrap()]
            )
            .is_err()
        );
        println!(
            "schema purge PG case {mode}: unrelated intent/history retained, scoped deletion complete, immutable audit preserved"
        );
    }
}
