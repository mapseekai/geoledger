#![allow(
    clippy::unwrap_used,
    reason = "test fixtures and assertions should fail immediately"
)]
use geoledger_engine::{Application, Storage};
use serde_json::{Value, json};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn large_feature_save_publish_merge_and_history_have_no_16k_limit() -> TestResult {
    let f = Fixture::new()?;
    let seed = f.ws(&f.alice);
    let properties = json!({"payload":"x".repeat(128 * 1024),"left":0,"right":0});
    f.save(&f.alice, &seed, 0, properties.clone(), point(1., 2.))?;
    assert_eq!(f.get(&f.alice, Some(&seed))["properties"], properties);
    f.publish(&f.alice, &seed, 1)?;
    let left = f.ws(&f.alice);
    let right = f.ws(&f.bob);
    let mut a = properties.clone();
    let mut b = properties.clone();
    a["left"] = json!(1);
    b["right"] = json!(1);
    f.save(&f.alice, &left, 0, a, point(1., 2.))?;
    f.save(&f.bob, &right, 0, b, point(1., 2.))?;
    f.publish(&f.alice, &left, 1)?;
    f.publish(&f.bob, &right, 1)?;
    let mut expected = properties.clone();
    expected["left"] = json!(1);
    expected["right"] = json!(1);
    assert_eq!(f.get(&f.alice, None)["properties"], expected);
    assert_eq!(
        f.call(&f.alice, "commit", json!({"revision":1}))?["changes"][0]["after"]["properties"],
        properties
    );
    Ok(())
}

#[test]
fn codec_rejects_reserved_keys_and_preserves_binary64_properties() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.alice);
    for key in [
        "$serde_json::private::RawValue",
        "$serde_json::private::Number",
    ] {
        let mut request = json!({"workspace":w,"expected_workspace_version":0,"edits":[f.edit("one",json!({}),Value::Null)]});
        // Insert directly: constructing the request through to_value can itself
        // interpret serde's private marker keys when features are unified.
        let marker = Value::Object(
            [(key.to_owned(), Value::String("123".into()))]
                .into_iter()
                .collect(),
        );
        request["edits"][0]["feature"]["properties"]["nested"] = Value::Array(vec![marker]);
        assert_eq!(f.call(&f.alice, "save", request).unwrap_err().status, 400);
        assert_eq!(
            f.call(&f.alice, "get_workspace", json!({"workspace":w}))?["version"],
            0
        );
    }
    let properties = json!({"decimal":30.384158368805974,"tiny":7.93377316861183e-76,"marker":"$serde_json::private::RawValue"});
    f.save(&f.alice, &w, 0, properties.clone(), Value::Null)?;
    assert_eq!(f.get(&f.alice, Some(&w))["properties"], properties);
    f.publish(&f.alice, &w, 1)?;
    assert_eq!(f.get(&f.alice, None)["properties"], properties);
    assert_eq!(
        f.call(&f.alice, "commit", json!({"revision":1}))?["changes"][0]["after"]["properties"],
        properties
    );
    Ok(())
}

#[test]
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
fn features_pagination_returns_a_cursor_only_when_the_snapshot_has_more_results() -> TestResult {
    let f = Fixture::new()?;
    let empty = f.ws(&f.alice);
    assert_feature_pages(&f, json!({}), &[], 0, None)?;
    assert_feature_pages(&f, json!({"workspace":empty}), &[], 0, Some(0))?;

    let ids: Vec<_> = (0..70).map(|i| format!("{i:03}")).collect();
    let edits: Vec<_> = ids
        .iter()
        .map(|id| f.edit(id, json!({}), Value::Null))
        .collect();
    f.call(
        &f.alice,
        "save",
        json!({"workspace":empty,"expected_workspace_version":0,"edits":edits}),
    )?;
    assert_feature_pages(&f, json!({"workspace":empty}), &ids, 0, Some(1))?;
    f.publish(&f.alice, &empty, 1)?;
    assert_feature_pages(&f, json!({"workspace":empty}), &ids, 1, Some(2))?;
    assert_feature_pages(&f, json!({}), &ids, 1, None)?;
    assert_feature_pages(&f, json!({"revision":1}), &ids, 1, None)?;
    Ok(())
}

#[test]
fn features_pagination_uses_bbox_matches_after_workspace_moves_and_deletes() -> TestResult {
    let f = Fixture::new()?;
    let seed = f.ws(&f.alice);
    let edits: Vec<_> = ["a", "b", "c", "d", "e", "f", "g", "h"]
        .into_iter()
        .map(|id| {
            let x = if matches!(id, "d" | "f") { 20. } else { 0. };
            f.edit(id, json!({}), point(x, x))
        })
        .collect();
    f.call(
        &f.alice,
        "save",
        json!({"workspace":seed,"expected_workspace_version":0,"edits":edits}),
    )?;
    f.publish(&f.alice, &seed, 1)?;
    let draft = f.ws(&f.alice);
    f.call(
        &f.alice,
        "save",
        json!({"workspace":draft,"expected_workspace_version":0,"edits":[
            f.edit("b",json!({}),point(20.,20.)),
            f.edit("bb",json!({}),point(0.,0.)),
            {"dataset":f.d,"feature_id":"c","feature":null},
            f.edit("d",json!({}),point(0.,0.)),
            {"dataset":f.d,"feature_id":"g","feature":null},
            f.edit("h",json!({}),point(20.,20.)),
            f.edit("z",json!({}),point(20.,20.))
        ]}),
    )?;
    let bounds = json!([-1., -1., 1., 1.]);
    let visible: Vec<_> = ["a", "bb", "d", "e"].map(String::from).into();
    assert_feature_pages(
        &f,
        json!({"workspace":draft,"bbox":bounds}),
        &visible,
        1,
        Some(1),
    )?;
    let all: Vec<_> = ["a", "b", "bb", "d", "e", "f", "h", "z"]
        .map(String::from)
        .into();
    assert_feature_pages(&f, json!({"workspace":draft}), &all, 1, Some(1))?;
    assert_feature_pages(
        &f,
        json!({"workspace":draft,"bbox":[40.,40.,41.,41.]}),
        &[],
        1,
        Some(1),
    )?;
    let pinned = f.ws(&f.alice);
    f.publish(&f.alice, &draft, 1)?;
    let original: Vec<_> = ["a", "b", "c", "e", "g", "h"].map(String::from).into();
    assert_feature_pages(&f, json!({"revision":1,"bbox":bounds}), &original, 1, None)?;
    assert_feature_pages(
        &f,
        json!({"workspace":pinned,"bbox":bounds}),
        &original,
        1,
        Some(0),
    )?;
    assert_feature_pages(&f, json!({"revision":2,"bbox":bounds}), &visible, 2, None)?;
    assert_feature_pages(&f, json!({"bbox":bounds}), &visible, 2, None)?;
    assert_feature_pages(
        &f,
        json!({"workspace":draft,"bbox":bounds}),
        &visible,
        2,
        Some(2),
    )?;
    assert_feature_pages(&f, json!({"workspace":draft}), &all, 2, Some(2))?;

    let later = f.ws(&f.alice);
    f.call(
        &f.alice,
        "save",
        json!({"workspace":later,"expected_workspace_version":0,"edits":[
            {"dataset":f.d,"feature_id":"bb","feature":null},
            f.edit("d",json!({"later":true}),point(20.,20.)),
            f.edit("aa",json!({}),point(0.,0.))
        ]}),
    )?;
    f.publish(&f.alice, &later, 1)?;
    assert_feature_pages(
        &f,
        json!({"workspace":draft,"bbox":bounds}),
        &visible,
        2,
        Some(2),
    )?;
    assert_feature_pages(&f, json!({"workspace":draft}), &all, 2, Some(2))?;
    let published = f.call(
        &f.alice,
        "features",
        json!({"dataset":f.d,"workspace":draft,"feature_id":"d"}),
    )?;
    assert_eq!(published["geometry"], point(0., 0.));
    assert_eq!(published["revision"], 2);
    assert_eq!(published["workspace_version"], 2);
    assert_eq!(
        f.call(&f.bob, "features", json!({"dataset":f.d,"workspace":draft}),)
            .unwrap_err()
            .status,
        404
    );
    assert_eq!(
        f.call(
            &f.alice,
            "features",
            json!({"dataset":f.d,"workspace":draft,"feature_id":"c"}),
        )
        .unwrap_err()
        .status,
        404
    );
    assert_eq!(
        f.call(&f.alice, "get_workspace", json!({"workspace":draft}))?["base_revision"],
        1
    );
    assert_eq!(
        f.call(&f.alice, "diff", json!({"workspace":draft}))?["base_revision"],
        1
    );
    f.call(
        &f.alice,
        "discard",
        json!({"workspace":pinned,"expected_workspace_version":0}),
    )?;
    assert_feature_pages(
        &f,
        json!({"workspace":pinned,"bbox":bounds}),
        &original,
        1,
        Some(1),
    )?;
    Ok(())
}

#[test]
fn features_pagination_keeps_workspace_versions_available_for_cross_page_checks() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.alice);
    f.call(
        &f.alice,
        "save",
        json!({"workspace":w,"expected_workspace_version":0,"edits":[
            f.edit("a",json!({}),Value::Null),f.edit("b",json!({}),Value::Null)
        ]}),
    )?;
    let first = f.call(
        &f.alice,
        "features",
        json!({"dataset":f.d,"workspace":w,"limit":1}),
    )?;
    assert_eq!(first["workspace_version"], 1);
    assert_eq!(first["next_after"], "a");
    f.call(
        &f.alice,
        "save",
        json!({"workspace":w,"expected_workspace_version":1,"edits":[f.edit("c",json!({}),Value::Null)]}),
    )?;
    let second = f.call(
        &f.alice,
        "features",
        json!({"dataset":f.d,"workspace":w,"after":first["next_after"],"limit":2}),
    )?;
    assert_eq!(second["revision"], first["revision"]);
    assert_eq!(second["workspace_version"], 2);
    assert_ne!(second["workspace_version"], first["workspace_version"]);
    assert_eq!(second["features"][0]["id"], "b");
    assert_eq!(second["features"][1]["id"], "c");
    assert_eq!(second["next_after"], Value::Null);
    Ok(())
}

#[test]
fn features_pagination_lookahead_does_not_charge_the_response_budget() -> TestResult {
    let mut f = Fixture::new()?;
    let w = f.ws(&f.alice);
    f.call(
        &f.alice,
        "save",
        json!({"workspace":w,"expected_workspace_version":0,"edits":[
            f.edit("a",json!({}),Value::Null),
            f.edit("b",json!({"payload":"x".repeat(128*1024)}),Value::Null)
        ]}),
    )?;
    f.publish(&f.alice, &w, 1)?;
    f.app = f.app.clone().with_response_limit(512);
    let page = f.call(&f.alice, "features", json!({"dataset":f.d,"limit":1}))?;
    assert_eq!(page["features"].as_array().unwrap().len(), 1);
    assert_eq!(page["features"][0]["id"], "a");
    assert_eq!(page["next_after"], "a");
    assert_eq!(
        f.call(
            &f.alice,
            "features",
            json!({"dataset":f.d,"limit":1,"after":page["next_after"]})
        )
        .unwrap_err()
        .status,
        413
    );
    let feature = f.call(
        &f.alice,
        "features",
        json!({"dataset":f.d,"feature_id":"a","limit":1}),
    )?;
    assert_eq!(feature["type"], "Feature");
    assert_eq!(feature["id"], "a");
    assert!(feature.get("next_after").is_none());
    Ok(())
}

#[test]
fn duplicate_dataset_is_conflict_and_rolls_back_without_public_diagnostics() -> TestResult {
    use std::error::Error as _;
    let f = Fixture::new()?;
    let datasets = f.call(&f.alice, "list_datasets", json!({}))?;
    let audit = f.call(&f.alice, "audit", json!({}))?;
    let error = f
        .call(
            &f.alice,
            "create_dataset",
            json!({"geometry_type":"point","name":"features"}),
        )
        .unwrap_err();
    assert!(error.source().is_some());
    assert_eq!(error.status, 409);
    assert_eq!(error.body["error"]["code"], "conflict");
    assert!(error.body["error"]["request_id"].is_string());
    assert!(!error.to_string().contains("duplicate key"));
    assert!(!error.to_string().contains("UNIQUE constraint"));
    assert_eq!(f.call(&f.alice, "list_datasets", json!({}))?, datasets);
    assert_eq!(f.call(&f.alice, "audit", json!({}))?, audit);
    // The rejected transaction must not poison the next connection or write.
    f.call(
        &f.alice,
        "create_dataset",
        json!({"geometry_type":"point","name":"after-conflict"}),
    )?;
    assert_eq!(
        f.call(&f.alice, "list_datasets", json!({}))?
            .as_array()
            .unwrap()
            .len(),
        2
    );
    Ok(())
}

#[test]
fn property_nul_is_rejected_consistently_without_rejecting_literal_escapes() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.alice);
    for properties in [
        json!({"value":"before\0after"}),
        json!({"nested":[{"array":["before\0after"]}]}),
        json!({"nested":[{"before\0after":true}]}),
        json!({"before\0after":true}),
    ] {
        let error = f
            .save(&f.alice, &w, 0, properties, Value::Null)
            .unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(error.body["error"]["code"], "invalid_argument");
        assert_eq!(
            f.call(&f.alice, "get_workspace", json!({"workspace":w}))?["version"],
            0
        );
    }
    let properties = json!({"value":"before\\u0000after","nested":[{"literal\\u0000":"allowed"}]});
    f.save(&f.alice, &w, 0, properties.clone(), Value::Null)?;
    f.publish(&f.alice, &w, 1)?;
    assert_eq!(f.get(&f.alice, None)["properties"], properties);
    Ok(())
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: Application,
    p: String,
    d: String,
    alice: String,
    bob: String,
    viewer: String,
}
impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let storage = if std::env::var("GL_CONFORMANCE_BACKEND").as_deref() == Ok("postgis") {
            let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
            let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
            let name: String = db.query_one("SELECT current_database()", &[])?.get(0);
            if name != "geoledger_test" {
                return Err("requires geoledger_test".into());
            }
            Storage::Postgis(dsn)
        } else {
            Storage::Sqlite(dir.path().join("geoledger.db"))
        };
        let app = Application::new(storage);
        app.migrate()?;
        app.migrate()?;
        let alice = Uuid::new_v4().to_string();
        let bob = Uuid::new_v4().to_string();
        let viewer = Uuid::new_v4().to_string();
        let p = app.execute(
            &alice,
            "create_project",
            json!({"name":"integration project"}),
        )?["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = app.execute(
            &alice,
            "create_dataset",
            json!({"geometry_type":"point","project":p,"name":"features"}),
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
            _dir: dir,
            app,
            p,
            d,
            alice,
            bob,
            viewer,
        })
    }
    fn call(&self, s: &str, op: &str, mut v: Value) -> Result<Value, geoledger_engine::Error> {
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
    ) -> Result<Value, geoledger_engine::Error> {
        self.call(s,"save",json!({"workspace":w,"expected_workspace_version":v,"edits":[self.edit("one",props,point)]}))
    }
    fn edit(&self, key: &str, props: Value, point: Value) -> Value {
        json!({"dataset":self.d,"feature_id":key,"feature":{"type":"Feature","id":key,"properties":props,"geometry":point}})
    }
    fn publish(&self, s: &str, w: &str, v: i64) -> Result<Value, geoledger_engine::Error> {
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

fn assert_feature_pages(
    f: &Fixture,
    mut request: Value,
    expected: &[String],
    revision: i64,
    workspace_version: Option<i64>,
) -> TestResult {
    request["dataset"] = json!(f.d);
    let mut limits = vec![expected.len().max(1), expected.len() + 1];
    if expected.len() > 32 {
        limits.extend([17, 32, 35]);
    } else {
        limits.extend([1, 2]);
    }
    limits.sort_unstable();
    limits.dedup();
    for limit in limits {
        request["after"] = json!("");
        request["limit"] = json!(limit);
        for start in (0..expected.len().max(1)).step_by(limit) {
            let end = (start + limit).min(expected.len());
            let page = f.call(&f.alice, "features", request.clone())?;
            let ids: Vec<_> = page["features"]
                .as_array()
                .unwrap()
                .iter()
                .map(|feature| feature["id"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(ids, expected[start..end], "request: {request}");
            assert_eq!(page["revision"], revision);
            assert_eq!(page["workspace_version"], json!(workspace_version));
            let next = (end < expected.len()).then(|| expected[end - 1].clone());
            assert_eq!(page["next_after"], json!(next), "request: {request}");
            request["after"] = page["next_after"].clone();
        }
    }
    // A caller can still explicitly seek beyond the final visible feature.
    request["after"] = json!(expected.last().map_or("", String::as_str));
    let empty = f.call(&f.alice, "features", request)?;
    assert_eq!(empty["features"], json!([]));
    assert_eq!(empty["next_after"], Value::Null);
    Ok(())
}

#[test]
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
    assert_eq!(
        f.call(&f.bob, "diff", json!({"workspace":b}))?["changes"][0]["draft"]["properties"],
        json!({"a":1,"b":2})
    );
    f.publish(&f.bob, &b, 1)?;
    assert_eq!(f.get(&f.alice, None)["properties"], json!({"a":2,"b":2}));
    assert_eq!(
        f.call(&f.bob, "diff", json!({"workspace":b}))?["changes"][0]["draft"]["properties"],
        json!({"a":2,"b":2})
    );
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
        f.call(
            &f.viewer,
            "create_dataset",
            json!({"geometry_type":"point","name":"denied"})
        )
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
        json!({"type":"Point","coordinates":[999]}),
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

#[test]
fn preserves_geometry_direction_z_and_requires_explicit_delete() -> TestResult {
    let mut f = Fixture::new()?;
    f.seed();
    f.d = f.call(
        &f.alice,
        "create_dataset",
        json!({"name":"lines","geometry_type":"line","coordinate_dimension":3}),
    )?["dataset"]
        .as_str()
        .unwrap()
        .to_owned();
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
fn stale_deletion_resolution_after_other_publication_is_reconfirmed() -> TestResult {
    deletion_resolution_requires_confirmation(true)
}

#[test]
fn stale_deletion_resolution_after_unrelated_save_is_reconfirmed() -> TestResult {
    deletion_resolution_requires_confirmation(false)
}

#[test]
fn change_pages_signal_exact_end_and_preserve_tuple_order() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.alice);
    let empty = f.call(&f.alice, "diff", json!({"workspace":w,"limit":2}))?;
    assert_eq!(empty.get("next_after"), Some(&Value::Null));
    assert_eq!(empty["changes"], json!([]));
    let keys: Vec<_> = (0..34).map(|i| format!("key/{i:02}")).collect();
    let edits: Vec<_> = keys
        .iter()
        .map(|k| f.edit(k, json!({"exact":9007199254740993_u64}), Value::Null))
        .collect();
    f.call(
        &f.alice,
        "save",
        json!({"workspace":w,"expected_workspace_version":0,"edits":edits}),
    )?;
    let expected: Vec<_> = keys.iter().map(|k| json!(format!("{}/{k}", f.d))).collect();
    assert_query_pages(
        &f,
        "diff",
        json!({"workspace":w}),
        ("changes", "cursor"),
        json!(""),
        &expected,
        &[1, 2, 17, 34, 35],
    )?;
    f.publish(&f.alice, &w, 1)?;
    assert_query_pages(
        &f,
        "commit",
        json!({"revision":1}),
        ("changes", "cursor"),
        json!(""),
        &expected,
        &[1, 2, 17, 34, 35],
    )?;
    Ok(())
}

#[test]
fn change_page_probe_does_not_consume_response_budget() -> TestResult {
    let mut f = Fixture::new()?;
    let w = f.ws(&f.alice);
    let edits = [
        f.edit("a", json!({"small":true}), Value::Null),
        f.edit("b", json!({"payload":"x".repeat(48 * 1024)}), Value::Null),
    ];
    f.call(
        &f.alice,
        "save",
        json!({"workspace":w,"expected_workspace_version":0,"edits":edits}),
    )?;
    f.app = f.app.clone().with_response_limit(2048);
    let page = f.call(&f.alice, "diff", json!({"workspace":w,"limit":1}))?;
    assert_eq!(page["changes"][0]["feature_id"], "a");
    assert!(page["next_after"].is_string());
    assert_eq!(
        f.call(&f.alice, "diff", json!({"workspace":w,"limit":2}))
            .unwrap_err()
            .status,
        413
    );
    // Publication reads the actual large edit; the small read budget applies
    // only to the paged query, not fixture publication.
    f.app = f.app.clone().with_response_limit(64 * 1024 * 1024);
    f.publish(&f.alice, &w, 1)?;
    f.app = f.app.clone().with_response_limit(2048);
    let page = f.call(&f.alice, "commit", json!({"revision":1,"limit":1}))?;
    assert_eq!(page["changes"][0]["feature_id"], "a");
    assert!(page["next_after"].is_string());
    assert_eq!(
        f.call(&f.alice, "commit", json!({"revision":1,"limit":2}))
            .unwrap_err()
            .status,
        413
    );
    Ok(())
}

#[test]
fn history_and_audit_pages_signal_current_snapshot_end() -> TestResult {
    let f = Fixture::new()?;
    let empty = f.call(&f.alice, "history", json!({"limit":2}))?;
    assert_eq!(empty, json!({"commits":[],"next_after":null}));
    for revision in 1..=2 {
        let w = f.ws(&f.alice);
        f.save(&f.alice, &w, 0, json!({"revision":revision}), Value::Null)?;
        f.publish(&f.alice, &w, 1)?;
    }
    assert_query_pages(
        &f,
        "history",
        json!({}),
        ("commits", "revision"),
        json!(0),
        &[json!(1), json!(2)],
        &[1, 2, 3],
    )?;
    let audit = f.call(&f.alice, "audit", json!({"limit":1000}))?;
    let ids: Vec<_> = audit["events"]
        .as_array()
        .ok_or("events")?
        .iter()
        .map(|e| e["id"].clone())
        .collect();
    assert!(ids.len() > 2);
    assert_query_pages(
        &f,
        "audit",
        json!({}),
        ("events", "id"),
        json!(0),
        &ids,
        &[1, 2, ids.len(), ids.len() + 1],
    )?;
    // A terminal page describes one request's snapshot. Later appends are visible
    // when polling after the last observed revision/event ID, even after a null cursor.
    let last_id = ids.last().ok_or("audit id")?;
    let w = f.ws(&f.alice);
    f.save(&f.alice, &w, 0, json!({"revision":3}), Value::Null)?;
    f.publish(&f.alice, &w, 1)?;
    let later = f.call(&f.alice, "history", json!({"after":2,"limit":2}))?;
    assert_eq!(later["commits"][0]["revision"], 3);
    assert_eq!(later["commits"].as_array().ok_or("commits")?.len(), 1);
    assert_eq!(later["next_after"], Value::Null);
    let later = f.call(&f.alice, "audit", json!({"after":last_id,"limit":1000}))?;
    let events = later["events"].as_array().ok_or("events")?;
    assert!(!events.is_empty());
    assert!(events.iter().all(|e| e["id"].as_i64() > last_id.as_i64()));
    assert_eq!(later["next_after"], Value::Null);
    Ok(())
}

fn assert_query_pages(
    f: &Fixture,
    operation: &str,
    input: Value,
    fields: (&str, &str),
    start: Value,
    expected: &[Value],
    limits: &[usize],
) -> TestResult {
    let (collection, cursor) = fields;
    for &limit in limits {
        let mut after = start.clone();
        let mut seen = Vec::new();
        loop {
            let mut request = input.clone();
            request["after"] = after.clone();
            request["limit"] = json!(limit);
            let page = f.call(&f.alice, operation, request)?;
            let rows = page[collection].as_array().ok_or("page collection")?;
            assert!(rows.len() <= limit);
            seen.extend(rows.iter().map(|row| row[cursor].clone()));
            assert_eq!(seen, expected[..seen.len()], "{operation} limit={limit}");
            let next = page.get("next_after").ok_or("page next_after")?;
            if seen.len() == expected.len() {
                assert_eq!(next, &Value::Null, "{operation} terminal limit={limit}");
                // Reading beyond the last returned item also yields an empty terminal page.
                let mut request = input.clone();
                request["after"] = rows.last().ok_or("last row")?[cursor].clone();
                request["limit"] = json!(limit);
                let empty = f.call(&f.alice, operation, request)?;
                assert_eq!(empty[collection], json!([]));
                assert_eq!(empty.get("next_after"), Some(&Value::Null));
                break;
            }
            assert_eq!(next, &rows.last().ok_or("last row")?[cursor]);
            assert_ne!(next, &after);
            after = next.clone();
        }
    }
    Ok(())
}

#[test]
fn audit_pagination_is_owner_only_and_project_scoped() -> TestResult {
    let f = Fixture::new()?;
    let page = f.call(&f.alice, "audit", json!({"limit":2}))?;
    let events = page["events"].as_array().ok_or("events")?;
    assert_eq!(events.len(), 2);
    let next = f.call(
        &f.alice,
        "audit",
        json!({"limit":2,"after":page["next_after"]}),
    )?;
    assert!(next["events"][0]["id"].as_i64().ok_or("id")? > events[1]["id"].as_i64().ok_or("id")?);
    for subject in [&f.bob, &f.viewer, "outsider"] {
        assert_eq!(f.call(subject, "audit", json!({})).unwrap_err().status, 404);
    }
    assert_eq!(
        f.call(&f.alice, "audit", json!({"after":-1}))
            .unwrap_err()
            .status,
        400
    );
    let other = Fixture::new()?;
    assert_eq!(
        f.app
            .execute(&f.alice, "audit", json!({"project":other.p}))
            .unwrap_err()
            .status,
        404
    );
    Ok(())
}

#[test]
fn membership_removal_archive_and_delete() -> TestResult {
    let f = Fixture::new()?;
    let status =
        |r: Result<Value, geoledger_engine::Error>| r.map(|_| 200).unwrap_or_else(|e| e.status);
    let members = f.call(&f.viewer, "list_members", json!({}))?;
    let mut listed: Vec<(String, String)> = members
        .as_array()
        .ok_or("members")?
        .iter()
        .map(|m| {
            (
                m["subject"].as_str().unwrap().into(),
                m["role"].as_str().unwrap().into(),
            )
        })
        .collect();
    listed.sort();
    let mut expected = vec![
        (f.alice.clone(), "owner".to_owned()),
        (f.bob.clone(), "editor".to_owned()),
        (f.viewer.clone(), "viewer".to_owned()),
    ];
    expected.sort();
    assert_eq!(listed, expected);
    assert_eq!(status(f.call("outsider", "list_members", json!({}))), 404);
    // Only owners remove others; members may leave; the last owner stays.
    assert_eq!(
        status(f.call(&f.bob, "remove_member", json!({"subject":f.viewer}))),
        404
    );
    f.call(&f.alice, "remove_member", json!({"subject":f.viewer}))?;
    assert_eq!(status(f.call(&f.viewer, "get_project", json!({}))), 404);
    assert_eq!(
        status(f.call(&f.viewer, "features", json!({"dataset":f.d}))),
        404
    );
    assert_eq!(
        status(f.call(&f.alice, "remove_member", json!({"subject":f.viewer}))),
        404
    );
    assert_eq!(
        f.call(&f.alice, "list_members", json!({}))?
            .as_array()
            .ok_or("m")?
            .len(),
        2
    );
    f.call(
        &f.alice,
        "set_member",
        json!({"subject":f.viewer,"role":"viewer"}),
    )?;
    assert_eq!(
        f.call(&f.viewer, "get_project", json!({}))?["role"],
        "viewer"
    );
    f.call(&f.viewer, "remove_member", json!({"subject":f.viewer}))?;
    assert_eq!(status(f.call(&f.viewer, "get_project", json!({}))), 404);
    assert_eq!(
        status(f.call(&f.alice, "remove_member", json!({"subject":f.alice}))),
        409
    );
    let audit = f.call(&f.alice, "audit", json!({"limit":1000}))?;
    assert!(
        audit["events"]
            .as_array()
            .ok_or("events")?
            .iter()
            .any(|e| e["action"] == "remove_member")
    );
    // Archived projects are read-only for data but stay readable.
    let w = f.seed();
    let archived = f.call(&f.alice, "archive_project", json!({"archived":true}))?;
    assert_eq!(archived["state"], "archived");
    assert_eq!(
        status(f.call(&f.bob, "archive_project", json!({"archived":false}))),
        404
    );
    assert_eq!(status(f.call(&f.bob, "create_workspace", json!({}))), 409);
    assert_eq!(
        status(f.save(&f.alice, &w, 2, json!({}), point(1., 1.))),
        409
    );
    assert_eq!(f.get(&f.bob, None)["properties"]["a"], 1);
    let projects = f.app.execute(&f.bob, "list_projects", json!({}))?;
    let listed = projects
        .as_array()
        .ok_or("projects")?
        .iter()
        .find(|p| p["project"] == f.p.as_str())
        .ok_or("listed")?
        .clone();
    assert_eq!(listed["state"], "archived");
    assert_eq!(listed["role"], "editor");
    f.call(&f.alice, "archive_project", json!({"archived":false}))?;
    f.ws(&f.bob);
    // Deletion needs the exact name, is owner-only and hides the project everywhere.
    assert_eq!(
        status(f.call(&f.alice, "delete_project", json!({"confirm_name":"wrong"}))),
        400
    );
    assert_eq!(
        status(f.call(
            &f.bob,
            "delete_project",
            json!({"confirm_name":"integration project"})
        )),
        404
    );
    f.call(
        &f.alice,
        "delete_project",
        json!({"confirm_name":"integration project"}),
    )?;
    for subject in [&f.alice, &f.bob] {
        assert_eq!(status(f.call(subject, "get_project", json!({}))), 404);
        assert_eq!(
            status(f.call(subject, "features", json!({"dataset":f.d}))),
            404
        );
        let projects = f.app.execute(subject, "list_projects", json!({}))?;
        assert!(
            projects
                .as_array()
                .ok_or("projects")?
                .iter()
                .all(|p| p["project"] != f.p.as_str())
        );
    }
    assert_eq!(
        status(f.call(
            &f.alice,
            "set_member",
            json!({"subject":f.bob,"role":"owner"})
        )),
        404
    );
    Ok(())
}

#[test]
fn platform_admins_creation_policy_and_quota() -> TestResult {
    let f = Fixture::new()?;
    let status =
        |r: Result<Value, geoledger_engine::Error>| r.map(|_| 200).unwrap_or_else(|e| e.status);
    f.app.bootstrap_admin("admin", "initial-test-password")?;
    let login = f.app.auth_operation(
        None,
        "login_password",
        json!({"username":"admin","password":"initial-test-password"}),
    )?;
    let root = login["user"]["id"].as_str().ok_or("admin id")?.to_owned();
    let carol = Uuid::new_v4().to_string();
    let restricted = f.app.clone().with_policy(geoledger_engine::Policy {
        admin_only_project_creation: true,
        max_owned_projects: Some(1),
    });
    assert_eq!(
        status(restricted.execute(&carol, "create_project", json!({"name":"x"}))),
        403
    );
    restricted.execute(&root, "create_project", json!({"name":"a"}))?;
    restricted.execute(&root, "create_project", json!({"name":"b"}))?;
    let quota = f.app.clone().with_policy(geoledger_engine::Policy {
        max_owned_projects: Some(1),
        ..Default::default()
    });
    quota.execute(&carol, "create_project", json!({"name":"first"}))?;
    assert_eq!(
        status(quota.execute(&carol, "create_project", json!({"name":"second"}))),
        403
    );
    // Administrators manage any project but gain no data access by doing so.
    let call = |op: &str, mut v: Value| {
        v["project"] = json!(f.p);
        restricted.execute(&root, op, v)
    };
    assert_eq!(call("get_project", json!({}))?["role"], "admin");
    assert_eq!(
        call("list_members", json!({}))?
            .as_array()
            .ok_or("m")?
            .len(),
        3
    );
    call("set_member", json!({"subject":carol,"role":"owner"}))?;
    call("remove_member", json!({"subject":f.alice}))?;
    assert_eq!(status(call("features", json!({"dataset":f.d}))), 404);
    assert_eq!(status(call("create_workspace", json!({}))), 404);
    assert_eq!(f.call(&carol, "get_project", json!({}))?["role"], "owner");
    let audit = f.call(&carol, "audit", json!({"limit":1000}))?;
    assert!(
        audit["events"]
            .as_array()
            .ok_or("events")?
            .iter()
            .any(|e| e["action"] == "set_member" && e["subject"] == root.as_str())
    );
    // Database administrators retain authorization across Application instances.
    assert_eq!(
        status(f.app.execute(&root, "get_project", json!({"project":f.p}))),
        200
    );
    Ok(())
}

#[test]
fn dataset_family_and_lifecycle_preserve_shared_history() -> TestResult {
    let f = Fixture::new()?;
    assert_eq!(
        f.call(&f.alice, "create_dataset", json!({"name":"missing type"}))
            .unwrap_err()
            .status,
        400
    );
    let other = f.call(
        &f.alice,
        "create_dataset",
        json!({"name":"areas","geometry_type":"polygon"}),
    )?["dataset"]
        .as_str()
        .unwrap()
        .to_owned();
    let polygon = json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]});
    let area_edit = json!({"dataset":other,"feature_id":"area","feature":{"type":"Feature","id":"area","properties":{},"geometry":polygon}});
    let mixed = f.ws(&f.alice);
    let invalid = json!({"workspace":mixed,"expected_workspace_version":0,"edits":[area_edit.clone(), f.edit("bad",json!({}), polygon.clone())]});
    assert_eq!(f.call(&f.alice, "save", invalid).unwrap_err().status, 400);
    assert_eq!(
        f.call(&f.alice, "diff", json!({"workspace":mixed}))?["changes"],
        json!([])
    );
    f.call(&f.alice,"save",json!({"workspace":mixed,"expected_workspace_version":0,"edits":[area_edit, f.edit("one",json!({}),point(1.,2.))]}))?;
    f.publish(&f.alice, &mixed, 1)?;
    let only = f.ws(&f.alice);
    f.save(&f.alice, &only, 0, json!({"changed":true}), point(1., 2.))?;
    f.publish(&f.alice, &only, 1)?;
    let draft = f.ws(&f.alice);
    f.save(&f.alice, &draft, 0, json!({}), point(3., 4.))?;
    let untouched = f.ws(&f.alice);
    assert_eq!(
        f.call(
            &f.bob,
            "delete_dataset",
            json!({"dataset":f.d,"confirm_name":"features"})
        )
        .unwrap_err()
        .status,
        404
    );
    assert_eq!(
        f.call(
            &f.alice,
            "delete_dataset",
            json!({"dataset":f.d,"confirm_name":"wrong"})
        )
        .unwrap_err()
        .status,
        400
    );
    f.call(
        &f.alice,
        "rename_project",
        json!({"name":"renamed project"}),
    )?;
    assert_eq!(
        f.call(
            &f.alice,
            "rename_dataset",
            json!({"dataset":f.d,"name":"renamed points"})
        )?["geometry_type"],
        "point"
    );
    f.call(
        &f.alice,
        "delete_dataset",
        json!({"dataset":f.d,"confirm_name":"renamed points"}),
    )?;
    let history = f.call(&f.alice, "history", json!({}))?;
    assert_eq!(history["commits"].as_array().unwrap().len(), 1);
    let changes = f.call(&f.alice, "commit", json!({"revision":1}))?;
    assert_eq!(changes["changes"].as_array().unwrap().len(), 1);
    assert_eq!(changes["changes"][0]["dataset"], other);
    assert_eq!(
        f.call(&f.alice, "get_workspace", json!({"workspace":mixed}))?["version"],
        3
    );
    for w in [&only, &draft] {
        assert_eq!(
            f.call(&f.alice, "get_workspace", json!({"workspace":w}))
                .unwrap_err()
                .status,
            404
        );
    }
    f.call(&f.alice, "get_workspace", json!({"workspace":untouched}))?;
    assert_eq!(
        f.call(&f.alice, "features", json!({"dataset":other}))?["features"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.call(
        &f.alice,
        "delete_project",
        json!({"confirm_name":"renamed project"}),
    )?;
    assert_eq!(
        f.call(&f.alice, "get_project", json!({}))
            .unwrap_err()
            .status,
        404
    );
    Ok(())
}

#[test]
fn workspace_over_thousand_features_can_publish_and_restore() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.alice);
    for batch in 0..11 {
        let edits: Vec<_> = (batch * 100..(batch * 100 + 100).min(1001))
            .map(|i| f.edit(&format!("bulk-{i:04}"), json!({"n":i}), Value::Null))
            .collect();
        f.call(
            &f.alice,
            "save",
            json!({"workspace":w,"expected_workspace_version":batch,"edits":edits}),
        )?;
    }
    assert_eq!(
        f.call(&f.alice, "workspace_summary", json!({"workspace":w}))?["total"]["added"],
        1001
    );
    let result = f.publish(&f.alice, &w, 11)?;
    assert_eq!(
        f.call(&f.viewer, "commit_summary", json!({"revision":1}))?["total"]["added"],
        1001
    );
    assert_eq!(result["changes"], 1001);
    let undo = f.call(&f.alice, "restore", json!({"revision":1}))?;
    let undo = undo["workspace"].as_str().ok_or("workspace")?;
    assert_eq!(f.publish(&f.alice, undo, 0)?["changes"], 1001);
    assert_eq!(
        f.call(&f.alice, "features", json!({"dataset":f.d}))?["features"],
        json!([])
    );
    assert_eq!(
        f.call(
            &f.alice,
            "features",
            json!({"dataset":f.d,"revision":1,"limit":1000})
        )?["features"]
            .as_array()
            .ok_or("features")?
            .len(),
        1000
    );
    Ok(())
}

#[test]
fn topology_warnings_preserve_original_polygon_through_publication() -> TestResult {
    let mut f = Fixture::new()?;
    f.d = f.call(
        &f.alice,
        "create_dataset",
        json!({"name":"raw boundaries","geometry_type":"polygon"}),
    )?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    let w = f.ws(&f.alice);
    let crossing = json!({"type":"Polygon","coordinates":[[[0,0],[1,1],[0,1],[1,0],[0,0]]]});
    let result = f.save(&f.alice, &w, 0, json!({}), crossing.clone())?;
    assert_eq!(result["warnings"].as_array().ok_or("warnings")?.len(), 1);
    assert!(
        result["warnings"][0]
            .as_str()
            .ok_or("warning")?
            .contains("self-intersection")
    );
    assert_eq!(f.get(&f.alice, Some(&w))["geometry"], crossing);
    f.publish(&f.alice, &w, 1)?;
    assert_eq!(f.get(&f.alice, None)["geometry"], crossing);
    let audit = f.call(&f.alice, "audit", json!({}))?;
    assert!(
        audit["events"]
            .as_array()
            .ok_or("audit")?
            .iter()
            .any(|event| event["detail"]["topology_warnings"]
                .as_array()
                .is_some_and(|w| !w.is_empty()))
    );
    Ok(())
}

#[test]
fn published_workspace_summary_keeps_its_base_after_concurrent_insert_or_delete() -> TestResult {
    for concurrent_insert in [true, false] {
        let f = Fixture::new()?;
        let seed = f.ws(&f.alice);
        f.call(
            &f.alice,
            "save",
            json!({"workspace":seed,"expected_workspace_version":0,"edits":[
                f.edit("changed",json!({"n":1}),Value::Null),
                f.edit("removed",json!({}),Value::Null)
            ]}),
        )?;
        f.publish(&f.alice, &seed, 1)?;
        let draft = f.ws(&f.alice);
        let competing = f.ws(&f.bob);
        f.call(
            &f.alice,
            "save",
            json!({"workspace":draft,"expected_workspace_version":0,"edits":[
                f.edit("added",json!({"n":2}),Value::Null),
                f.edit("changed",json!({"n":2}),Value::Null),
                {"dataset":f.d,"feature_id":"removed","feature":null}
            ]}),
        )?;
        let concurrent_edit = if concurrent_insert {
            f.edit("added", json!({"n":1}), Value::Null)
        } else {
            json!({"dataset":f.d,"feature_id":"changed","feature":null})
        };
        f.call(
            &f.bob,
            "save",
            json!({"workspace":competing,"expected_workspace_version":0,"edits":[concurrent_edit]}),
        )?;
        f.publish(&f.bob, &competing, 1)?;
        let conflict_key = if concurrent_insert {
            "added"
        } else {
            "changed"
        };
        f.call(
            &f.alice,
            "resolve",
            json!({"workspace":draft,"expected_workspace_version":1,"expected_head":2,
                "resolutions":[f.edit(conflict_key,json!({"n":3}),Value::Null)]}),
        )?;
        let before = f.call(&f.alice, "diff", json!({"workspace":draft}))?;
        assert_eq!(before["base_revision"], 1);
        let expected = json!({"added":1,"deleted":1,"modified":1});
        assert_eq!(
            f.call(&f.alice, "workspace_summary", json!({"workspace":draft}))?["total"],
            expected
        );
        f.publish(&f.alice, &draft, 2)?;
        let after = f.call(&f.alice, "diff", json!({"workspace":draft}))?;
        assert_eq!(after["base_revision"], before["base_revision"]);
        assert_eq!(after["changes"], before["changes"]);
        let summary = f.call(&f.alice, "workspace_summary", json!({"workspace":draft}))?;
        assert_eq!(summary["version"], 3);
        assert_eq!(summary["total"], expected);
        assert_eq!(
            f.call(&f.bob, "workspace_summary", json!({"workspace":draft}))
                .unwrap_err()
                .status,
            404
        );
        let commit = f.call(&f.alice, "commit", json!({"revision":3}))?;
        assert_eq!(commit["changes"][0]["before"].is_null(), !concurrent_insert);
        assert_eq!(commit["changes"][1]["before"].is_null(), !concurrent_insert);
        let commit_summary = f.call(&f.alice, "commit_summary", json!({"revision":3}))?;
        assert_eq!(
            commit_summary["total"],
            if concurrent_insert {
                json!({"added":0,"deleted":1,"modified":2})
            } else {
                json!({"added":2,"deleted":1,"modified":0})
            }
        );

        // Later current rows must not change either historical classification.
        let later = f.ws(&f.alice);
        f.call(
            &f.alice,
            "save",
            json!({"workspace":later,"expected_workspace_version":0,"edits":[
                {"dataset":f.d,"feature_id":"added","feature":null},
                f.edit("changed",json!({"n":4}),Value::Null),
                f.edit("removed",json!({"restored":true}),Value::Null)
            ]}),
        )?;
        f.publish(&f.alice, &later, 1)?;
        assert_eq!(
            f.call(&f.alice, "workspace_summary", json!({"workspace":draft}))?,
            summary
        );
        assert_eq!(f.call(&f.alice, "diff", json!({"workspace":draft}))?, after);
        assert_eq!(
            f.call(&f.alice, "commit_summary", json!({"revision":3}))?,
            commit_summary
        );
    }
    Ok(())
}

#[test]
fn summaries_count_mixed_datasets_and_follow_rename_delete_and_authorization() -> TestResult {
    let f = Fixture::new()?;
    let seed = f.ws(&f.alice);
    f.call(&f.alice,"save",json!({"workspace":seed,"expected_workspace_version":0,"edits":[f.edit("removed",json!({}),Value::Null),f.edit("changed",json!({"n":1}),Value::Null)]}))?;
    f.publish(&f.alice, &seed, 1)?;
    let d2 = f.call(
        &f.alice,
        "create_dataset",
        json!({"name":"other","geometry_type":"point"}),
    )?["dataset"]
        .as_str()
        .unwrap()
        .to_owned();
    let w = f.ws(&f.alice);
    assert_eq!(
        f.call(&f.alice, "workspace_summary", json!({"workspace":w}))?["datasets"],
        json!([])
    );
    let mut other = f.edit("new", json!({"payload":"x".repeat(128*1024)}), Value::Null);
    other["dataset"] = json!(d2);
    f.call(&f.alice,"save",json!({"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":f.d,"feature_id":"removed","feature":null},f.edit("changed",json!({"n":2}),Value::Null),other]}))?;
    let draft = f.call(&f.alice, "workspace_summary", json!({"workspace":w}))?;
    assert_eq!(draft["version"], 1);
    assert_eq!(draft["total"], json!({"added":1,"deleted":1,"modified":1}));
    assert_eq!(draft["datasets"].as_array().unwrap().len(), 2);
    assert!(draft.to_string().len() < 700);
    assert!(
        f.call("outsider", "workspace_summary", json!({"workspace":w}))
            .is_err()
    );
    f.publish(&f.alice, &w, 1)?;
    let published = f.app.clone().with_response_limit(2048).execute(
        &f.alice,
        "workspace_summary",
        json!({"project":f.p,"workspace":w}),
    )?;
    assert_eq!(published["total"], draft["total"]);
    assert_eq!(published["datasets"], draft["datasets"]);
    let commit = f.call(&f.viewer, "commit_summary", json!({"revision":2}))?;
    assert_eq!(commit["total"], draft["total"]);
    assert_eq!(commit["datasets"], draft["datasets"]);
    assert!(
        f.call("outsider", "commit_summary", json!({"revision":2}))
            .is_err()
    );
    assert!(
        f.call(&f.alice, "commit_summary", json!({"revision":999}))
            .is_err()
    );
    f.call(
        &f.alice,
        "rename_dataset",
        json!({"dataset":d2,"name":"renamed"}),
    )?;
    let renamed = f.call(&f.viewer, "commit_summary", json!({"revision":2}))?;
    assert!(
        renamed["datasets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "renamed")
    );
    f.call(
        &f.alice,
        "delete_dataset",
        json!({"dataset":d2,"confirm_name":"renamed"}),
    )?;
    let remaining = f.call(&f.viewer, "commit_summary", json!({"revision":2}))?;
    assert_eq!(
        remaining["total"],
        json!({"added":0,"deleted":1,"modified":1})
    );
    assert_eq!(remaining["datasets"].as_array().unwrap().len(), 1);
    let remaining_workspace = f.call(&f.alice, "workspace_summary", json!({"workspace":w}))?;
    assert_eq!(remaining_workspace["total"], remaining["total"]);
    assert_eq!(remaining_workspace["datasets"], remaining["datasets"]);
    Ok(())
}

#[test]
fn native_geometry_preserves_nested_collections_and_empty_features() -> TestResult {
    let mut f = Fixture::new()?;
    f.d = f.call(
        &f.alice,
        "create_dataset",
        json!({"name":"xyz","geometry_type":"point","coordinate_dimension":3}),
    )?["dataset"]
        .as_str()
        .unwrap()
        .to_owned();
    for geometry in [
        json!({"type":"GeometryCollection","geometries":[{"type":"MultiPoint","coordinates":[[1,2,3],[4,5,6]]},{"type":"Point","coordinates":[7,8,9]}]}),
        json!({"type":"MultiPoint","coordinates":[]}),
        json!({"type":"GeometryCollection","geometries":[]}),
        json!({"type":"GeometryCollection","geometries":[{"type":"MultiPoint","coordinates":[]},{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,2,3]}]}]}),
    ] {
        let w = f.ws(&f.alice);
        f.save(&f.alice, &w, 0, json!({}), geometry.clone())
            .inspect_err(|error| {
                let mut cause = std::error::Error::source(error);
                while let Some(e) = cause {
                    eprintln!("geometry {geometry}: {e:?}");
                    cause = e.source();
                }
            })?;
        assert_eq!(f.get(&f.alice, Some(&w))["geometry"], geometry);
        f.publish(&f.alice, &w, 1)?;
        assert_eq!(f.get(&f.alice, None)["geometry"], geometry);
    }
    let w = f.ws(&f.alice);
    let mixed = json!({"type":"MultiPoint","coordinates":[[1,2],[3,4,5]]});
    assert_eq!(
        f.save(&f.alice, &w, 0, json!({}), mixed)
            .unwrap_err()
            .status,
        400
    );
    Ok(())
}

#[test]
fn datasets_fix_coordinate_dimension_and_reject_mismatched_edits_atomically() -> TestResult {
    let mut f = Fixture::new()?;
    let datasets = f.call(&f.alice, "list_datasets", json!({}))?;
    assert_eq!(datasets[0]["coordinate_dimension"], 2);
    assert_eq!(
        f.call(
            &f.alice,
            "create_dataset",
            json!({"name":"bad dimension","geometry_type":"point","coordinate_dimension":4})
        )
        .unwrap_err()
        .status,
        400
    );
    let w = f.ws(&f.alice);
    assert_eq!(
        f.save(
            &f.alice,
            &w,
            0,
            json!({}),
            json!({"type":"Point","coordinates":[1,2,3]})
        )
        .unwrap_err()
        .status,
        400
    );
    assert_eq!(
        f.call(&f.alice, "get_workspace", json!({"workspace":w}))?["version"],
        0
    );
    f.save(&f.alice, &w, 0, json!({}), point(1., 2.))?;
    f.publish(&f.alice, &w, 1)?;
    let xyz = f.call(
        &f.alice,
        "create_dataset",
        json!({"name":"three dimensional","geometry_type":"point","coordinate_dimension":3}),
    )?;
    assert_eq!(xyz["coordinate_dimension"], 3);
    f.d = xyz["dataset"].as_str().unwrap().to_owned();
    let w = f.ws(&f.alice);
    assert_eq!(
        f.save(&f.alice, &w, 0, json!({}), point(1., 2.))
            .unwrap_err()
            .status,
        400
    );
    let geometry = json!({"type":"Point","coordinates":[1,2,3.123456789012345]});
    f.save(&f.alice, &w, 0, json!({}), geometry.clone())?;
    f.publish(&f.alice, &w, 1)?;
    assert_eq!(f.get(&f.alice, None)["geometry"], geometry);
    let renamed = f.call(
        &f.alice,
        "rename_dataset",
        json!({"dataset":f.d,"name":"renamed xyz"}),
    )?;
    assert_eq!(renamed["coordinate_dimension"], 3);
    Ok(())
}

#[test]
fn receipt_replay_survives_archive_and_downgrade_without_granting_new_writes() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.bob);
    f.save(&f.bob, &w, 0, json!({"a":1}), Value::Null)?;
    let request = json!({"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"receipt"});
    let result = f.call(&f.bob, "publish", request.clone())?;
    f.call(&f.alice, "archive_project", json!({"archived":true}))?;
    assert_eq!(f.call(&f.bob, "publish", request.clone())?, result);
    assert_eq!(
        f.call(&f.bob, "create_workspace", json!({}))
            .unwrap_err()
            .status,
        409
    );
    let mut mismatch = request.clone();
    mismatch["message"] = json!("changed");
    assert_eq!(f.call(&f.bob, "publish", mismatch).unwrap_err().status, 409);
    f.call(
        &f.alice,
        "set_member",
        json!({"subject":f.bob,"role":"viewer"}),
    )?;
    assert_eq!(f.call(&f.bob, "publish", request.clone())?, result);
    f.call(&f.alice, "remove_member", json!({"subject":f.bob}))?;
    assert_eq!(f.call(&f.bob, "publish", request).unwrap_err().status, 404);
    Ok(())
}

#[test]
fn derived_draft_count_tracks_upserts_deletions_and_failed_batches() -> TestResult {
    let f = Fixture::new()?;
    let w = f.ws(&f.alice);
    assert_eq!(
        f.save(&f.alice, &w, 0, json!({"a":1}), Value::Null)?["changes"],
        1
    );
    assert_eq!(
        f.save(&f.alice, &w, 1, json!({"a":2}), Value::Null)?["changes"],
        1
    );
    let remove = json!({"dataset":f.d,"feature_id":"one","feature":null});
    assert_eq!(
        f.call(
            &f.alice,
            "save",
            json!({"workspace":w,"expected_workspace_version":2,"edits":[remove]})
        )?["changes"],
        0
    );
    let bad = f.edit(
        "bad",
        json!({}),
        json!({"type":"Point","coordinates":[999]}),
    );
    assert!(f.call(&f.alice,"save",json!({"workspace":w,"expected_workspace_version":3,"edits":[f.edit("two",json!({}),Value::Null),bad]})).is_err());
    assert_eq!(
        f.save(&f.alice, &w, 3, json!({"a":3}), Value::Null)?["changes"],
        1
    );
    Ok(())
}

#[test]
fn buffered_saves_preserve_values_and_rollback_flushed_batches() -> TestResult {
    for payload_size in [0, 70_000] {
        let f = Fixture::new()?;
        let w = f.ws(&f.alice);
        let edits: Vec<_> = (0..40)
            .map(|i| {
                f.edit(
            &format!("batch-{i:02}"),
            json!({"row":i,"exact":9007199254740993_u64,"payload":"x".repeat(payload_size)}),
            if i % 2 == 0 { point(1., 2.) } else { Value::Null },
        )
            })
            .collect();
        let mut invalid = edits.clone();
        invalid[39]["feature"]["geometry"] = json!({"type":"Point","coordinates":[999]});
        assert!(
            f.call(
                &f.alice,
                "save",
                json!({"workspace":w,"expected_workspace_version":0,"edits":invalid})
            )
            .is_err()
        );
        assert_eq!(
            f.call(&f.alice, "get_workspace", json!({"workspace":w}))?["version"],
            0
        );
        let query = json!({"workspace":w,"dataset":f.d,"limit":100});
        assert!(
            f.call(&f.alice, "features", query.clone())?["features"]
                .as_array()
                .ok_or("features")?
                .is_empty()
        );
        assert_eq!(
            f.call(
                &f.alice,
                "save",
                json!({"workspace":w,"expected_workspace_version":0,"edits":edits})
            )?["changes"],
            40
        );
        let actual = f.call(&f.alice, "features", query)?;
        let values = actual["features"].as_array().ok_or("features")?;
        assert_eq!(values.len(), edits.len());
        for (actual, edit) in values.iter().zip(&edits) {
            assert_eq!(actual["properties"], edit["feature"]["properties"]);
            assert_eq!(actual["geometry"], edit["feature"]["geometry"]);
        }
        f.publish(&f.alice, &w, 1)?;
        let unchanged = f.ws(&f.alice);
        assert_eq!(
            f.call(
                &f.alice,
                "save",
                json!({"workspace":unchanged,"expected_workspace_version":0,"edits":edits})
            )?["changes"],
            0
        );
    }
    Ok(())
}

#[test]
fn native_crs_roundtrips_and_spatial_filters_remain_dataset_scoped() -> TestResult {
    let mut f = Fixture::new()?;
    for crs in [
        "EPSG:3857",
        "EPSG:4490",
        "EPSG:32650",
        "LOCAL:survey",
        "+proj=tmerc +lat_0=0 +lon_0=117 +ellps=GRS80 +units=m",
    ] {
        let d = f.call(
            &f.alice,
            "create_dataset",
            json!({"name":crs,"geometry_type":"point","crs":crs}),
        )?;
        assert_eq!(d["crs"], crs);
        f.d = d["dataset"].as_str().unwrap().into();
        let w = f.ws(&f.alice);
        let g = point(500000.123456789, 3500000.987654321);
        f.save(
            &f.alice,
            &w,
            0,
            json!({"exact":9007199254740993_u64}),
            g.clone(),
        )?;
        let bbox = json!([499999, 3499999, 500001, 3500001]);
        let draft = f.call(
            &f.alice,
            "features",
            json!({"dataset":f.d,"workspace":w,"bbox":bbox}),
        )?;
        assert_eq!(draft["crs"], crs);
        assert_eq!(draft["features"].as_array().unwrap().len(), 1);
        assert_eq!(draft["features"][0]["geometry"], g);
        f.publish(&f.alice, &w, 1)?;
        let published = f.call(&f.alice, "features", json!({"dataset":f.d,"bbox":bbox}))?;
        assert_eq!(published["features"], draft["features"]);
        assert!(
            f.call(
                &f.alice,
                "features",
                json!({"dataset":f.d,"bbox":[0,0,1,1]})
            )?["features"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let renamed = f.call(
            &f.alice,
            "rename_dataset",
            json!({"dataset":f.d,"name":format!("native {crs}")}),
        )?;
        assert_eq!(renamed["crs"], crs);
        assert!(
            f.call(
                &f.alice,
                "rename_dataset",
                json!({"dataset":f.d,"name":"invalid change","crs":"EPSG:4326"})
            )
            .is_err()
        );
    }
    Ok(())
}
