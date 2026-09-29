#![allow(clippy::unwrap_used)]
use geoledger_center::CenterApplication;
use serde_json::{Value, json};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn thousand_large_features_publish_and_page_conflicts_without_full_materialization() -> TestResult {
    let mut f = Fixture::new()?;
    // This is a large debug-build correctness/memory workload, not the
    // production latency SLA. Deadline behavior has dedicated short tests.
    f.app = f
        .app
        .clone()
        .with_timeout(std::time::Duration::from_secs(180));
    let seed = f.ws(&f.alice);
    let save_all = |subject: &str, workspace: &str, x: i64| -> TestResult {
        for batch in 0..10 {
            let edits: Vec<_> = (batch * 100..(batch + 1) * 100)
                .map(|i| {
                    f.edit(
                        &format!("large-{i:04}"),
                        json!({"array":vec![0;7000],"x":x,"exact":9007199254740993_u64}),
                        point(1., 2.),
                    )
                })
                .collect();
            f.call(
                subject,
                "save",
                json!({"workspace":workspace,"expected_workspace_version":batch,"edits":edits}),
            )?;
        }
        Ok(())
    };
    save_all(&f.alice, &seed, 0)?;
    assert_eq!(f.publish(&f.alice, &seed, 10)?["changes"], 1000);
    let left = f.ws(&f.alice);
    let right = f.ws(&f.bob);
    save_all(&f.alice, &left, 1)?;
    save_all(&f.bob, &right, 2)?;
    let diff = f.call(&f.bob, "diff", json!({"workspace":right,"limit":1}))?;
    assert_eq!(diff["changes"].as_array().unwrap().len(), 1);
    assert_eq!(
        diff["changes"][0]["draft"]["properties"]["exact"].as_u64(),
        Some(9007199254740993)
    );
    assert_eq!(f.publish(&f.alice, &left, 10)?["changes"], 1000);
    let page = f.call(&f.bob, "conflicts", json!({"workspace":right,"limit":1}))?;
    assert_eq!(page["total"], 1000);
    assert_eq!(page["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(page["truncated"], true);
    let next = f.call(
        &f.bob,
        "conflicts",
        json!({"workspace":right,"limit":1,"after":page["next_after"]}),
    )?;
    assert_ne!(
        next["conflicts"][0]["feature_id"],
        page["conflicts"][0]["feature_id"]
    );
    // Classification must include conflicts beyond the retained response page.
    let resolved = f.call(&f.bob,"resolve",json!({"workspace":right,"expected_workspace_version":10,"expected_head":2,"resolutions":[f.edit("large-0999",json!({"x":3}),point(1.,2.))]}))?;
    assert_eq!(resolved["remaining_conflicts"], 999);
    let conflict = f.publish(&f.bob, &right, 11).unwrap_err();
    assert_eq!(conflict.status, 409);
    assert_eq!(conflict.body["total"], 999);
    assert_eq!(f.call(&f.alice, "get_project", json!({}))?["head"], 2);
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn database_errors_retain_source_without_public_diagnostics() -> TestResult {
    use std::error::Error as _;
    let f = Fixture::new()?;
    let error = f
        .call(&f.alice, "create_dataset", json!({"name":"features"}))
        .unwrap_err();
    assert!(error.source().is_some());
    assert_eq!(error.body["error"]["code"], "unavailable");
    assert!(error.body["error"]["request_id"].is_string());
    assert!(!error.to_string().contains("duplicate key"));
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn write_and_commit_deadlines_allow_atomic_idempotent_retry() -> TestResult {
    for deferred in [false, true] {
        let f = Fixture::new()?;
        let w = f.ws(&f.alice);
        f.save(
            &f.alice,
            &w,
            0,
            json!({"exact":18446744073709551615_u64}),
            point(1., 2.),
        )?;
        let mut control = test_connection()?;
        let name = format!("deadline_{}", Uuid::new_v4().simple());
        let table = if deferred { "commits" } else { "features" };
        let trigger = if deferred {
            "CREATE CONSTRAINT TRIGGER"
        } else {
            "CREATE TRIGGER"
        };
        let timing = if deferred {
            "AFTER INSERT"
        } else {
            "BEFORE INSERT"
        };
        let deferral = if deferred {
            "DEFERRABLE INITIALLY DEFERRED"
        } else {
            ""
        };
        control.batch_execute(&format!("CREATE FUNCTION _geoledger_center.{name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.project='{}'::uuid THEN PERFORM pg_sleep(1); END IF; RETURN NEW; END $$; {trigger} {name} {timing} ON _geoledger_center.{table} {deferral} FOR EACH ROW EXECUTE FUNCTION _geoledger_center.{name}();",f.p))?;
        let payload = json!({"project":f.p,"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"deadline retry"});
        let timed = f
            .app
            .clone()
            .with_timeout(std::time::Duration::from_millis(250));
        let error = timed
            .execute(&f.alice, "publish", payload.clone())
            .unwrap_err();
        assert_eq!(error.status, 504);
        // Waits for the isolated backend to leave the test trigger and release
        // its table lock. A COMMIT reply may be lost, so retry the original ID.
        control.batch_execute(&format!("DROP TRIGGER {name} ON _geoledger_center.{table}; DROP FUNCTION _geoledger_center.{name}();"))?;
        let published = f.app.execute(&f.alice, "publish", payload.clone())?;
        assert_eq!(published["revision"], 1);
        assert_eq!(f.app.execute(&f.alice, "publish", payload)?, published);
        assert_eq!(
            f.get(&f.alice, None)["properties"]["exact"].as_u64(),
            Some(u64::MAX)
        );
    }
    Ok(())
}
struct Fixture {
    app: CenterApplication,
    p: String,
    d: String,
    alice: String,
    bob: String,
    viewer: String,
}
impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        // ONLY this variable is read. The guard runs before migrations or project creation.
        let app = CenterApplication::new(std::env::var("GL_TEST_DATABASE_URL")?);
        app.check_test_database()?;
        app.migrate()?;
        app.migrate()?;
        let alice = Uuid::new_v4().to_string();
        let bob = Uuid::new_v4().to_string();
        let viewer = Uuid::new_v4().to_string();
        let p = app.execute(
            &alice,
            "create_project",
            json!({"name":"center integration"}),
        )?["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = app.execute(
            &alice,
            "create_dataset",
            json!({"project":p,"name":"features"}),
        )?["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        for (subject, role) in [(&bob, "editor"), (&viewer, "viewer")] {
            app.execute(
                &alice,
                "set_member",
                json!({"project":p,"subject":subject,"role":role}),
            )?;
        }
        Ok(Self {
            app,
            p,
            d,
            alice,
            bob,
            viewer,
        })
    }
    fn call(&self, s: &str, op: &str, mut v: Value) -> Result<Value, geoledger_center::Error> {
        v["project"] = json!(self.p);
        self.app.execute(s, op, v)
    }
    fn ws(&self, s: &str) -> String {
        self.call(s, "create_workspace", json!({})).unwrap()["workspace"]
            .as_str()
            .unwrap()
            .into()
    }
    fn save(
        &self,
        s: &str,
        w: &str,
        v: i64,
        props: Value,
        point: Value,
    ) -> Result<Value, geoledger_center::Error> {
        self.call(s,"save",json!({"workspace":w,"expected_workspace_version":v,"edits":[self.edit("one",props,point)]}))
    }
    fn edit(&self, key: &str, props: Value, point: Value) -> Value {
        json!({"dataset":self.d,"feature_id":key,"feature":{"type":"Feature","id":key,"properties":props,"geometry":point}})
    }
    fn publish(&self, s: &str, w: &str, v: i64) -> Result<Value, geoledger_center::Error> {
        self.call(s,"publish",json!({"workspace":w,"expected_workspace_version":v,"request_id":Uuid::new_v4(),"message":"test"}))
    }
    fn get(&self, s: &str, w: Option<&str>) -> Value {
        self.call(
            s,
            "features",
            json!({"dataset":self.d,"workspace":w,"feature_id":"one"}),
        )
        .unwrap()
    }
    fn seed(&self) -> String {
        let w = self.ws(&self.alice);
        self.save(
            &self.alice,
            &w,
            0,
            json!({"a":1,"b":1,"nullable":null}),
            point(0., 0.),
        )
        .unwrap();
        self.publish(&self.alice, &w, 1).unwrap();
        w
    }
}
fn point(x: f64, y: f64) -> Value {
    json!({"type":"Point","coordinates":[
        if x.fract() == 0.0 { json!(x as i64) } else { json!(x) },
        if y.fract() == 0.0 { json!(y as i64) } else { json!(y) }
    ]})
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn merge_pinned_isolation_conflict_rebase_and_restore() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let unedited = f.ws(&f.bob);
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    f.save(
        &f.alice,
        &a,
        0,
        json!({"a":2,"b":1,"nullable":null}),
        point(0., 0.),
    )?;
    f.save(&f.bob, &b, 0, json!({"a":1,"b":2}), point(0., 0.))?;
    f.publish(&f.alice, &a, 1)?;
    assert_eq!(f.get(&f.bob, Some(&unedited))["properties"]["a"], 1);
    assert_eq!(f.get(&f.bob, Some(&unedited))["workspace_version"], 0);
    assert_eq!(f.get(&f.bob, Some(&b))["properties"], json!({"a":1,"b":2}));
    f.publish(&f.bob, &b, 1)?;
    assert_eq!(f.get(&f.alice, None)["properties"], json!({"a":2,"b":2}));
    let historical = f.call(
        &f.alice,
        "features",
        json!({"dataset":f.d,"revision":1,"feature_id":"one"}),
    )?;
    assert_eq!(historical["properties"]["a"], 1);
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    f.save(&f.alice, &a, 0, json!({"a":3,"b":2}), point(0., 0.))?;
    f.save(&f.bob, &b, 0, json!({"a":4,"b":2}), point(0., 0.))?;
    f.publish(&f.alice, &a, 1)?;
    let err = f.publish(&f.bob, &b, 1).unwrap_err();
    assert_eq!(err.status, 409);
    assert_eq!(err.body["conflicts"][0]["fields"], json!(["/properties/a"]));
    assert_eq!(err.body["head"], 4);
    assert_eq!(err.body["version"], 1);
    assert_eq!(
        f.call(&f.bob, "get_workspace", json!({"workspace":b}))?["status"],
        "open"
    );
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 3);
    let resolution = f.edit("one", json!({"a":5,"b":2}), point(0., 0.));
    let rebase = json!({"workspace":b,"expected_head":4,"expected_workspace_version":1,"resolutions":[resolution]});
    let mut invalid = rebase.clone();
    invalid["resolutions"] = json!([]);
    assert_eq!(f.call(&f.bob, "rebase", invalid).unwrap_err().status, 409);
    let mut invalid = rebase.clone();
    invalid["expected_head"] = json!(3);
    assert_eq!(f.call(&f.bob, "rebase", invalid).unwrap_err().status, 409);
    let out = f.call(&f.bob, "rebase", rebase.clone())?;
    assert_eq!(out["version"], 2);
    assert_eq!(f.call(&f.bob, "rebase", rebase).unwrap_err().status, 409);
    f.publish(&f.bob, &b, 2)?;
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 5);
    let undo = f.call(&f.alice, "restore", json!({"revision":5}))?;
    let w = undo["workspace"].as_str().unwrap();
    assert_eq!(f.get(&f.alice, Some(w))["properties"]["a"], 3);
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 5);
    f.publish(&f.alice, w, 0)?;
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 3);
    let detail = f.call(&f.alice, "commit", json!({"revision":5}))?;
    assert_eq!(detail["changes"][0]["before"]["properties"]["a"], 3);
    assert_eq!(detail["changes"][0]["after"]["properties"]["a"], 5);
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn authorization_idempotency_batch_rollback_and_draft_race() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.alice);
    assert_eq!(
        f.call(&f.bob, "get_workspace", json!({"workspace":w}))
            .unwrap_err()
            .status,
        404
    );
    assert_eq!(
        f.call(&f.viewer, "create_workspace", json!({}))
            .unwrap_err()
            .status,
        404
    );
    assert_eq!(
        f.call(&f.viewer, "create_dataset", json!({"name":"denied"}))
            .unwrap_err()
            .status,
        404
    );
    assert_eq!(
        f.call("foreign", "get_project", json!({}))
            .unwrap_err()
            .status,
        404
    );
    assert_eq!(
        f.call(
            &f.bob,
            "set_member",
            json!({"subject":"foreign","role":"owner"})
        )
        .unwrap_err()
        .status,
        404
    );
    let other = Fixture::new()?;
    assert_eq!(f.call(&f.alice,"save",json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":other.d,"feature_id":"x","feature":null}]})).unwrap_err().status,404);
    let invalid = f.edit(
        "two",
        json!({}),
        json!({"type":"Point","coordinates":[999,0]}),
    );
    assert!(f.call(&f.alice,"save",json!({"workspace":w,"expected_workspace_version":0,"edits":[f.edit("one",json!({"a":1}),Value::Null),invalid]})).is_err());
    assert_eq!(
        f.call(&f.alice, "get_workspace", json!({"workspace":w}))?["version"],
        0
    );
    assert!(
        f.call(&f.alice, "diff", json!({"workspace":w}))?["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let (left, right) = std::thread::scope(|scope| {
        let x = scope.spawn(|| f.save(&f.alice, &w, 0, json!({"a":1}), Value::Null));
        let y = scope.spawn(|| f.save(&f.alice, &w, 0, json!({"a":2}), Value::Null));
        (x.join().unwrap(), y.join().unwrap())
    });
    assert_ne!(left.is_ok(), right.is_ok());
    assert_eq!(left.err().or(right.err()).unwrap().status, 409);
    let request = json!({"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"once"});
    let a = f.call(&f.alice, "publish", request.clone())?;
    let b = f.call(&f.alice, "publish", request.clone())?;
    assert_eq!(a, b);
    let mut changed = request;
    changed["message"] = json!("different");
    assert_eq!(
        f.call(&f.alice, "publish", changed).unwrap_err().status,
        409
    );
    let drafts = f.call(&f.bob, "list_workspaces", json!({}))?;
    assert!(drafts.as_array().unwrap().is_empty());
    assert_eq!(f.get(&f.viewer, None)["id"], "one");
    assert_eq!(
        f.call(&f.alice, "create_workspace", json!({"author":"forged"}))
            .unwrap_err()
            .status,
        400
    );
    let w = f.ws(&f.bob);
    f.save(&f.bob, &w, 0, json!({"a":5}), Value::Null)?;
    f.call(
        &f.alice,
        "set_member",
        json!({"subject":f.bob,"role":"viewer"}),
    )?;
    assert_eq!(f.publish(&f.bob, &w, 1).unwrap_err().status, 404);
    assert_eq!(
        f.call(
            &f.bob,
            "discard",
            json!({"workspace":w,"expected_workspace_version":1})
        )
        .unwrap_err()
        .status,
        404
    );
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn geometry_bbox_overlay_and_atomic_conflicts() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let w = f.ws(&f.alice);
    f.save(
        &f.alice,
        &w,
        0,
        json!({"a":1,"b":1,"nullable":null}),
        point(20., 20.),
    )?;
    let query = |bbox: Value| {
        f.call(
            &f.alice,
            "features",
            json!({"dataset":f.d,"workspace":w,"bbox":bbox}),
        )
    };
    assert!(
        query(json!([-1, -1, 1, 1]))?["features"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        query(json!([19, 19, 21, 21]))?["features"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.save(
        &f.alice,
        &w,
        1,
        json!({"a":1,"b":1,"nullable":null}),
        point(0.5, 0.5),
    )?;
    assert_eq!(
        query(json!([-1, -1, 1, 1]))?["features"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.call(&f.alice,"save",json!({"workspace":w,"expected_workspace_version":2,"edits":[{"dataset":f.d,"feature_id":"one","feature":null}]}))?;
    assert!(
        query(json!([-1, -1, 1, 1]))?["features"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.get(&f.alice, None)["geometry"], point(0., 0.));
    f.call(
        &f.alice,
        "discard",
        json!({"workspace":w,"expected_workspace_version":3}),
    )?;
    assert_eq!(f.get(&f.alice, None)["geometry"], point(0., 0.));
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    f.save(
        &f.alice,
        &a,
        0,
        json!({"a":1,"b":1,"nullable":null}),
        point(1., 1.),
    )?;
    f.save(
        &f.bob,
        &b,
        0,
        json!({"a":1,"b":1,"nullable":null}),
        point(2., 2.),
    )?;
    f.publish(&f.alice, &a, 1)?;
    let err = f.publish(&f.bob, &b, 1).unwrap_err();
    assert_eq!(err.body["conflicts"][0]["fields"], json!(["/geometry"]));
    let invalid = json!({"type":"Polygon","coordinates":[[[0,0],[1,1],[0,1],[1,0],[0,0]]]});
    assert_eq!(
        f.save(&f.bob, &b, 1, json!({}), invalid)
            .unwrap_err()
            .status,
        400
    );
    assert_eq!(
        f.call(&f.bob, "get_workspace", json!({"workspace":b}))?["version"],
        1
    );
    let invalid = json!({"type":"Point","coordinates":[0,0,0,0]});
    assert_eq!(
        f.save(&f.bob, &b, 1, json!({}), invalid)
            .unwrap_err()
            .status,
        400
    );
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn concurrent_publish_and_stale_head_resolution() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    f.save(
        &f.alice,
        &a,
        0,
        json!({"a":2,"b":1,"nullable":null}),
        point(0., 0.),
    )?;
    f.save(
        &f.bob,
        &b,
        0,
        json!({"a":1,"b":2,"nullable":null}),
        point(0., 0.),
    )?;
    let (x, y) = std::thread::scope(|scope| {
        let x = scope.spawn(|| f.publish(&f.alice, &a, 1));
        let y = scope.spawn(|| f.publish(&f.bob, &b, 1));
        (x.join().unwrap(), y.join().unwrap())
    });
    x?;
    y?;
    assert_eq!(
        f.get(&f.alice, None)["properties"],
        json!({"a":2,"b":2,"nullable":null})
    );
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    f.save(&f.alice, &a, 0, json!({"a":3}), Value::Null)?;
    f.save(&f.bob, &b, 0, json!({"a":4}), Value::Null)?;
    f.publish(&f.alice, &a, 1)?;
    let conflict = f.publish(&f.bob, &b, 1).unwrap_err();
    let latest = f.ws(&f.alice);
    f.save(&f.alice, &latest, 0, json!({"a":6}), Value::Null)?;
    f.publish(&f.alice, &latest, 1)?;
    let result=f.call(&f.bob,"rebase",json!({"workspace":b,"expected_workspace_version":1,"expected_head":conflict.body["head"],"resolutions":[f.edit("one",json!({"a":5}),Value::Null)]}));
    assert_eq!(result.unwrap_err().status, 409);
    assert_eq!(f.get(&f.bob, Some(&b))["properties"], json!({"a":4}));
    assert_eq!(
        f.call(&f.bob, "get_workspace", json!({"workspace":b}))?["version"],
        1
    );
    Ok(())
}

fn test_connection() -> Result<postgres::Client, Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let tls = postgres_native_tls::MakeTlsConnector::new(native_tls::TlsConnector::new()?);
    let mut c = postgres::Client::connect(&dsn, tls)?;
    let name: String = c.query_one("SELECT current_database()", &[])?.get(0);
    assert_eq!(name, "geoledger_test");
    Ok(c)
}
#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn publish_transaction_rollback_and_immutable_history() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let w = f.ws(&f.alice);
    f.call(&f.alice,"save",json!({"workspace":w,"expected_workspace_version":0,"edits":[f.edit("one",json!({"a":10}),Value::Null),f.edit("two",json!({"a":20}),Value::Null)]}))?;
    let mut c = test_connection()?;
    // Scoped to this random project; other tests can continue using the shared schema.
    let constraint = format!("test_{}", Uuid::new_v4().simple());
    c.batch_execute(&format!("ALTER TABLE _geoledger_center.features ADD CONSTRAINT {constraint} CHECK(project <> '{}'::uuid OR feature_id <> 'two') NOT VALID",f.p))?;
    let failed = f.publish(&f.alice, &w, 1);
    c.batch_execute(&format!(
        "ALTER TABLE _geoledger_center.features DROP CONSTRAINT {constraint}"
    ))?;
    assert!(failed.is_err());
    assert_eq!(f.call(&f.alice, "get_project", json!({}))?["head"], 1);
    assert_eq!(
        f.call(&f.alice, "get_workspace", json!({"workspace":w}))?["version"],
        1
    );
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 1);
    for table in ["commits", "history", "idempotency"] {
        let count: i64 = c
            .query_one(
                &format!(
                    "SELECT count(*) FROM _geoledger_center.{table} WHERE project=$1::text::uuid"
                ),
                &[&f.p],
            )?
            .get(0);
        assert_eq!(count, 1);
    }
    f.publish(&f.alice, &w, 1)?;
    let count: i64 = c
        .query_one(
            "SELECT count(*) FROM _geoledger_center.features WHERE project=$1::text::uuid",
            &[&f.p],
        )?
        .get(0);
    assert_eq!(count, 2);
    assert!(
        c.execute(
            "UPDATE _geoledger_center.history SET properties='{}' WHERE project=$1::text::uuid",
            &[&f.p]
        )
        .is_err()
    );
    assert!(
        c.execute(
            "DELETE FROM _geoledger_center.commits WHERE project=$1::text::uuid",
            &[&f.p]
        )
        .is_err()
    );
    assert!(c.execute("UPDATE _geoledger_center.commit_changes SET after_value=NULL WHERE project=$1::text::uuid",&[&f.p]).is_err());
    let old = f.call(
        &f.alice,
        "features",
        json!({"dataset":f.d,"feature_id":"one","revision":1}),
    )?;
    assert_eq!(old["properties"]["a"], 1);
    Ok(())
}
#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn http_identity_is_selected_only_by_server_token_mapping() -> TestResult {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let f = Fixture::new()?;
    let token = Uuid::new_v4().to_string() + &Uuid::new_v4().to_string();
    let tokens = geoledger_center::Tokens::from_json(
        json!([{"token":token,"subject":f.bob}])
            .to_string()
            .as_bytes(),
    )?;
    let router = geoledger_center::router(f.app.clone(), tokens);
    let runtime = tokio::runtime::Runtime::new()?;
    let response = runtime.block_on(
        router.clone().oneshot(
            Request::post("/api/center/create_project")
                .header("authorization", format!("Bearer {token}"))
                .header("x-subject", &f.alice)
                .body(Body::from(r#"{"name":"http identity"}"#))?,
        ),
    )?;
    assert_eq!(response.status(), 200);
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    let bytes = runtime.block_on(response.into_body().collect())?.to_bytes();
    let result: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        f.app
            .execute(&f.bob, "get_project", json!({"project":result["project"]}))?["role"],
        "owner"
    );
    assert_eq!(
        f.app
            .execute(
                &f.alice,
                "get_project",
                json!({"project":result["project"]})
            )
            .unwrap_err()
            .status,
        404
    );
    let response = runtime.block_on(
        router.oneshot(
            Request::post("/api/center/create_project")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"name":"http identity","author":"forged"}"#))?,
        ),
    )?;
    assert_eq!(response.status(), 400);
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn preserves_geometry_direction_z_and_requires_explicit_delete() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let w = f.ws(&f.alice);
    let omitted = json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":f.d,"feature_id":"one"}]});
    assert_eq!(f.call(&f.alice, "save", omitted).unwrap_err().status, 400);
    assert_eq!(
        f.call(&f.alice, "get_workspace", json!({"workspace":w}))?["version"],
        0
    );
    let line = json!({"type":"LineString","coordinates":[[121.003,31.01,2.5],[121,31,1]]});
    f.save(
        &f.alice,
        &w,
        0,
        json!({"name":"directed road"}),
        line.clone(),
    )?;
    assert_eq!(f.get(&f.alice, Some(&w))["geometry"], line);
    f.publish(&f.alice, &w, 1)?;
    assert_eq!(f.get(&f.alice, None)["geometry"], line);
    let w = f.ws(&f.alice);
    f.call(&f.alice,"save",json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":f.d,"feature_id":"one","feature":null}]}))?;
    f.publish(&f.alice, &w, 1)?;
    assert!(
        f.call(
            &f.alice,
            "features",
            json!({"dataset":f.d,"feature_id":"one"})
        )
        .is_err()
    );
    let prior = f.call(
        &f.alice,
        "features",
        json!({"dataset":f.d,"feature_id":"one","revision":2}),
    )?;
    assert_eq!(prior["geometry"], line);
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn merged_features_retain_the_same_validation_limits_as_saved_features() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    let base = json!({"a":1,"b":1,"nullable":null});
    let mut left = base.clone();
    let mut right = base;
    for i in 0..140 {
        left[format!("left_{i}")] = json!(i);
        right[format!("right_{i}")] = json!(i);
    }
    f.save(&f.alice, &a, 0, left.clone(), point(0., 0.))?;
    f.save(&f.bob, &b, 0, right, point(0., 0.))?;
    f.publish(&f.alice, &a, 1)?;
    assert_eq!(f.publish(&f.bob, &b, 1).unwrap_err().status, 422);
    assert_eq!(f.call(&f.alice, "get_project", json!({}))?["head"], 2);
    assert_eq!(f.get(&f.alice, None)["properties"], left);
    assert_eq!(
        f.call(&f.bob, "get_workspace", json!({"workspace":b}))?["status"],
        "open"
    );
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn draft_save_and_publish_avoid_foreign_key_lock_deadlock() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let w = f.ws(&f.alice);
    let mut control = test_connection()?;
    let key = i64::from(Uuid::new_v4().as_bytes()[0]) + 981000;
    let name = format!("review_{}", Uuid::new_v4().simple());
    // The fixture pauses save AFTER it owns the workspace lock. Publication
    // then waits for that workspace while owning only NO KEY UPDATE on project.
    control.execute("SELECT pg_advisory_lock($1)", &[&key])?;
    control.batch_execute(&format!(
        "CREATE FUNCTION _geoledger_center.{name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.project='{}'::uuid THEN PERFORM pg_advisory_xact_lock({key}::bigint); END IF; RETURN NEW; END $$; CREATE TRIGGER {name} BEFORE INSERT ON _geoledger_center.workspace_changes FOR EACH ROW EXECUTE FUNCTION _geoledger_center.{name}();",f.p
    ))?;
    let (saved, published) = std::thread::scope(|scope| {
        let save = scope.spawn(|| f.save(&f.alice, &w, 0, json!({"a":10}), point(0., 0.)));
        let mut blocked = false;
        for _ in 0..100 {
            let waiting: bool = control.query_one("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=$1::bigint::oid AND NOT granted)",&[&key]).unwrap().get(0);
            if waiting {
                blocked = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let publish = scope.spawn(|| f.publish(&f.alice, &w, 1));
        std::thread::sleep(std::time::Duration::from_millis(100));
        control
            .execute("SELECT pg_advisory_unlock($1)", &[&key])
            .unwrap();
        let saved = save.join().unwrap();
        let published = publish.join().unwrap();
        assert!(blocked, "save must reach fixture barrier");
        (saved, published)
    });
    control.batch_execute(&format!("DROP TRIGGER {name} ON _geoledger_center.workspace_changes; DROP FUNCTION _geoledger_center.{name}();"))?;
    saved?;
    published?;
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 10);
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn paged_conflicts_allow_incremental_resolution_and_reject_stale_choices() -> TestResult {
    let f = Fixture::new()?;
    let seed = f.ws(&f.alice);
    f.call(&f.alice,"save",json!({"workspace":seed,"expected_workspace_version":0,"edits":[f.edit("one",json!({"a":1}),Value::Null),f.edit("two",json!({"a":1}),Value::Null)]}))?;
    f.publish(&f.alice, &seed, 1)?;
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    for (subject, w, value) in [(&f.alice, &a, 2), (&f.bob, &b, 3)] {
        f.call(subject,"save",json!({"workspace":w,"expected_workspace_version":0,"edits":[f.edit("one",json!({"a":value}),Value::Null),f.edit("two",json!({"a":value}),Value::Null)]}))?;
    }
    f.publish(&f.alice, &a, 1)?;
    let first = f.call(&f.bob, "conflicts", json!({"workspace":b,"limit":1}))?;
    assert_eq!(first["total"], 2);
    assert_eq!(first["truncated"], true);
    let second = f.call(
        &f.bob,
        "conflicts",
        json!({"workspace":b,"limit":1,"after":first["next_after"]}),
    )?;
    assert_eq!(second["conflicts"][0]["feature_id"], "two");
    let resolve = json!({"workspace":b,"expected_workspace_version":1,"expected_head":2,"resolutions":[f.edit("one",json!({"a":4}),Value::Null)]});
    let result = f.call(&f.bob, "resolve", resolve.clone())?;
    assert_eq!(result["remaining_conflicts"], 1);
    assert_eq!(result["version"], 2);
    assert_eq!(f.call(&f.bob, "resolve", resolve).unwrap_err().status, 409);
    let err = f.publish(&f.bob, &b, 2).unwrap_err();
    assert_eq!(err.status, 409);
    assert_eq!(err.body["total"], 1);
    assert_eq!(err.body["conflicts"][0]["feature_id"], "two");
    f.call(&f.bob,"resolve",json!({"workspace":b,"expected_workspace_version":2,"expected_head":2,"resolutions":[f.edit("two",json!({"a":4}),Value::Null)]}))?;
    // A different publication makes every old resolution pin stale.
    let other = f.ws(&f.alice);
    f.save(&f.alice, &other, 0, json!({"a":5}), Value::Null)?;
    f.publish(&f.alice, &other, 1)?;
    let err = f.publish(&f.bob, &b, 3).unwrap_err();
    assert_eq!(err.status, 409);
    assert_eq!(err.body["head"], 3);
    f.call(&f.bob,"rebase",json!({"workspace":b,"expected_workspace_version":3,"expected_head":3,"resolutions":[f.edit("one",json!({"a":6}),Value::Null),f.edit("two",json!({"a":6}),Value::Null)]}))?;
    f.publish(&f.bob, &b, 4)?;
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 6);
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn saving_new_edits_invalidates_old_resolution_pins() -> TestResult {
    let f = Fixture::new()?;
    f.seed();
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    f.save(&f.alice, &a, 0, json!({"a":2}), Value::Null)?;
    f.save(&f.bob, &b, 0, json!({"a":3}), Value::Null)?;
    f.publish(&f.alice, &a, 1)?;
    f.call(&f.bob,"resolve",json!({"workspace":b,"expected_workspace_version":1,"expected_head":2,"resolutions":[f.edit("one",json!({"a":4}),Value::Null)]}))?;
    f.call(&f.bob,"save",json!({"workspace":b,"expected_workspace_version":2,"edits":[f.edit("new",json!({}),Value::Null)]}))?;
    let err = f.publish(&f.bob, &b, 3).unwrap_err();
    assert_eq!(err.status, 409);
    assert_eq!(err.body["conflicts"][0]["feature_id"], "one");
    assert_eq!(f.get(&f.alice, None)["properties"]["a"], 2);
    Ok(())
}

fn deletion_resolution_requires_confirmation(after_publish: bool) -> TestResult {
    let f = Fixture::new()?;
    let a = f.ws(&f.alice);
    let b = f.ws(&f.bob);
    f.save(&f.alice, &a, 0, json!({"value":"alice"}), point(0., 0.))?;
    f.save(&f.bob, &b, 0, json!({"value":"bob"}), point(0., 0.))?;
    f.publish(&f.alice, &a, 1)?;
    assert_eq!(f.publish(&f.bob, &b, 1).unwrap_err().status, 409);
    let deletion = json!({"dataset":f.d,"feature_id":"one","feature":null});
    f.call(&f.bob, "resolve", json!({"workspace":b,"expected_workspace_version":1,"expected_head":1,"resolutions":[deletion]}))?;
    let (head, version) = if after_publish {
        let other = f.ws(&f.alice);
        f.call(&f.alice, "save", json!({"workspace":other,"expected_workspace_version":0,"edits":[f.edit("two",json!({"value":"unrelated"}),point(1.,1.))]}))?;
        f.publish(&f.alice, &other, 1)?;
        (2, 2)
    } else {
        f.call(&f.bob, "save", json!({"workspace":b,"expected_workspace_version":2,"edits":[f.edit("two",json!({"value":"unrelated"}),point(1.,1.))]}))?;
        (1, 3)
    };
    let rejected = f.publish(&f.bob, &b, version).unwrap_err();
    assert_eq!(rejected.status, 409);
    assert_eq!(rejected.body["conflicts"][0]["reason"], "stale_resolution");
    assert_eq!(f.get(&f.alice, None)["properties"]["value"], "alice");
    f.call(&f.bob, "resolve", json!({"workspace":b,"expected_workspace_version":version,"expected_head":head,"resolutions":[deletion]}))?;
    f.publish(&f.bob, &b, version + 1)?;
    let rows = f.call(&f.alice, "features", json!({"dataset":f.d}))?;
    assert_eq!(rows["features"].as_array().unwrap().len(), 1);
    assert_eq!(rows["features"][0]["id"], "two");
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn stale_deletion_resolution_after_other_publication_is_reconfirmed() -> TestResult {
    deletion_resolution_requires_confirmation(true)
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn stale_deletion_resolution_after_unrelated_save_is_reconfirmed() -> TestResult {
    deletion_resolution_requires_confirmation(false)
}
