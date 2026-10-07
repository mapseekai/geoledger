#![allow(clippy::unwrap_used)]
use geoledger_center::{
    CenterApplication, CollaborationCommand, Error, Host, Scope, TableBinding, Transaction,
};
use serde_json::{Value, json};
use uuid::Uuid;

struct TestHost {
    table: String,
    fail: bool,
}
impl Host for TestHost {
    fn authorize(
        &self,
        _: &mut Transaction<'_>,
        scope: &Scope,
        _: bool,
    ) -> Result<TableBinding, Error> {
        if scope.tenant != "test" {
            return Err(Error::new(403, "denied"));
        }
        Ok(TableBinding {
            schema: "public".into(),
            table: self.table.clone(),
            id_column: "id".into(),
            geometry_column: "geom".into(),
            srid: 3857,
        })
    }
    fn published(
        &self,
        t: &mut Transaction<'_>,
        _: &Scope,
        revision: i64,
        _: bool,
    ) -> Result<(), Error> {
        t.execute(
            &format!("UPDATE public.{}_marker SET revision=$1", self.table),
            &[&revision],
        )?;
        if self.fail {
            return Err(Error::new(422, "host rejects publication"));
        }
        Ok(())
    }
}

#[test]
fn protocol_rejects_forged_identity_and_accepts_empty_message() {
    let request = json!({"op":"publish","epoch":Uuid::new_v4(),"workspace":Uuid::new_v4(),"expected_version":1,"request_id":Uuid::new_v4()});
    assert!(serde_json::from_value::<CollaborationCommand>(request.clone()).is_ok());
    let mut forged = request;
    forged["subject"] = json!("forged");
    assert!(serde_json::from_value::<CollaborationCommand>(forged).is_err());
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn creation_rejects_reused_ids_retains_defaults_and_allows_constraint_repair()
-> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let app = CenterApplication::new(dsn.clone());
    app.check_test_database()?;
    app.migrate()?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    let table = format!("collab_{}", Uuid::new_v4().simple());
    db.batch_execute(&format!("CREATE TABLE public.{table}(id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,a integer NOT NULL UNIQUE DEFAULT 7,geom geometry(Point,3857)); CREATE TABLE public.{table}_marker(revision bigint); INSERT INTO public.{table}_marker VALUES(0); INSERT INTO public.{table}(id,a) OVERRIDING SYSTEM VALUE VALUES(1,1);"))?;
    let host = TestHost {
        table: table.clone(),
        fail: false,
    };
    let scope = Scope {
        subject: "alice".into(),
        tenant: "test".into(),
        project: "project".into(),
        dataset: table.clone(),
    };
    let call = |v: Value| app.collaborate(&scope, serde_json::from_value(v).unwrap(), &host);
    let epoch = call(json!({"op":"register"}))?["epoch"].clone();
    let open = |base: &str| {
        call(json!({"op":"open_draft","epoch":epoch,"base_revision":base,"request_id":Uuid::new_v4()})).unwrap()["workspace"].clone()
    };
    let post = |workspace: &Value, properties: Value, geometry: Value| {
        call(
            json!({"op":"save_delta","epoch":epoch,"workspace":workspace,"expected_version":0,"request_id":Uuid::new_v4(),"operations":[{"method":"post","client_id":"new","body":{"type":"Feature","properties":properties,"geometry":geometry}}]}),
        )
    };
    let publish = |workspace: &Value, version: i64| {
        call(
            json!({"op":"publish","epoch":epoch,"workspace":workspace,"expected_version":version,"request_id":Uuid::new_v4()}),
        )
    };
    let collision = open("0");
    assert_eq!(
        post(&collision, json!({"a":2}), Value::Null)
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(
        db.query_one(&format!("SELECT a FROM public.{table} WHERE id=1"), &[])?
            .get::<_, i32>(0),
        1
    );
    assert_eq!(
        call(json!({"op":"draft_changes","epoch":epoch,"workspace":collision}))?["version"],
        0
    );
    let original = open("0");
    let original_saved = post(&original, json!({}), Value::Null)?;
    call(
        json!({"op":"rebase","epoch":epoch,"workspace":original,"expected_version":1,"expected_head":"0","request_id":Uuid::new_v4()}),
    )?;
    let changes = call(json!({"op":"draft_changes","epoch":epoch,"workspace":original}))?;
    assert!(
        changes["changes"][0]["feature"]["properties"]
            .get("a")
            .is_none()
    );
    let mut properties = changes["changes"][0]["feature"]["properties"].clone();
    properties.as_object_mut().unwrap().remove("id");
    let rebuilt = open("0");
    let saved = post(&rebuilt, properties, Value::Null)?;
    publish(&rebuilt, 1)?;
    let id = saved["ids"]["new"].as_str().unwrap();
    assert_eq!(
        db.query_one(
            &format!("SELECT a FROM public.{table} WHERE id::text=$1"),
            &[&id]
        )?
        .get::<_, i32>(0),
        7
    );
    let invalid = open("1");
    let saved = post(&invalid, json!({"a":null}), Value::Null)?;
    assert_eq!(publish(&invalid, 1).unwrap_err().status, 422);
    assert_eq!(call(json!({"op":"head"}))?["revision"], "1");
    assert_eq!(
        db.query_one(&format!("SELECT revision FROM public.{table}_marker"), &[])?
            .get::<_, i64>(0),
        1
    );
    call(
        json!({"op":"save_delta","epoch":epoch,"workspace":invalid,"expected_version":1,"request_id":Uuid::new_v4(),"operations":[{"method":"patch","id":saved["ids"]["new"],"body":{"properties":{"a":3}}}]}),
    )?;
    publish(&invalid, 2)?;
    let duplicate = open("2");
    post(&duplicate, json!({"a":3}), Value::Null)?;
    assert_eq!(publish(&duplicate, 1).unwrap_err().status, 422);
    let wrong_geometry = open("2");
    post(
        &wrong_geometry,
        json!({"a":4}),
        json!({"type":"LineString","coordinates":[[0,0],[1,1]]}),
    )?;
    assert_eq!(publish(&wrong_geometry, 1).unwrap_err().status, 422);
    assert_eq!(call(json!({"op":"head"}))?["revision"], "2");
    let deleted = open("2");
    call(
        json!({"op":"save_delta","epoch":epoch,"workspace":deleted,"expected_version":0,"request_id":Uuid::new_v4(),"operations":[{"method":"delete","id":"1"}]}),
    )?;
    publish(&deleted, 1)?;
    for used in [
        1_i64,
        original_saved["ids"]["new"]
            .as_str()
            .unwrap()
            .parse::<i64>()?,
    ] {
        db.query_one(
            "SELECT setval(pg_get_serial_sequence($1,'id')::regclass,$2::bigint,false)",
            &[&format!("public.{table}"), &used],
        )?;
        let attempted = open("3");
        assert_eq!(
            post(&attempted, json!({"a":9}), Value::Null)
                .unwrap_err()
                .status,
            409
        );
    }
    assert_eq!(call(json!({"op":"head"}))?["revision"], "3");
    db.batch_execute(&format!(
        "DROP TABLE public.{table}; DROP TABLE public.{table}_marker"
    ))?;
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn conflict_pages_are_contiguous_across_byte_limits_and_schema_order()
-> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let app = CenterApplication::new(dsn.clone()).with_timeout(std::time::Duration::from_secs(180));
    app.check_test_database()?;
    app.migrate()?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    let table = format!("collab_{}", Uuid::new_v4().simple());
    db.batch_execute(&format!("CREATE TABLE public.{table}(id text PRIMARY KEY,a integer,b integer,label text,geom geometry(Geometry,3857)); CREATE TABLE public.{table}_marker(revision bigint); INSERT INTO public.{table}_marker VALUES(0); INSERT INTO public.{table}(id,a,b,label) VALUES('!a',1,1,repeat('x',240000)),('b',1,1,repeat('x',180000)),('c',1,1,repeat('x',30000));"))?;
    let host = TestHost {
        table: table.clone(),
        fail: false,
    };
    let scope = Scope {
        subject: "alice".into(),
        tenant: "test".into(),
        project: "project".into(),
        dataset: table.clone(),
    };
    let call = |v: Value| app.collaborate(&scope, serde_json::from_value(v).unwrap(), &host);
    let epoch = call(json!({"op":"register"}))?["epoch"].clone();
    let open = || {
        call(json!({"op":"open_draft","epoch":epoch,"base_revision":"0","request_id":Uuid::new_v4()})).unwrap()["workspace"].clone()
    };
    let local = open();
    let remote = open();
    for (workspace, value) in [(&local, 2), (&remote, 3)] {
        let mut operations = ["!a", "b", "c"]
            .into_iter()
            .map(|id| json!({"method":"patch","id":id,"body":{"properties":{"a":value,"b":value}}}))
            .collect::<Vec<_>>();
        if value == 2 {
            operations.push(json!({"method":"patch","schema":true,"body":{"drop":["b"]}}));
        }
        call(
            json!({"op":"save_delta","epoch":epoch,"workspace":workspace,"expected_version":0,"request_id":Uuid::new_v4(),"operations":operations}),
        )?;
    }
    call(
        json!({"op":"publish","epoch":epoch,"workspace":remote,"expected_version":1,"request_id":Uuid::new_v4()}),
    )?;
    let preview = |after: Value, limit: usize| {
        call(json!({"op":"preview","epoch":epoch,"workspace":local,"after":after,"limit":limit}))
    };
    let ids = |page: &Value| {
        page["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let first = preview(Value::Null, 10)?;
    assert_eq!(first["total"], 4);
    assert_eq!(ids(&first), vec!["!a", "$schema"]);
    let second = preview(first["next_after"].clone(), 10)?;
    assert_eq!(ids(&second), vec!["b", "c"]);
    assert!(second["next_after"].is_null());
    let mut after = Value::Null;
    let mut seen = Vec::new();
    loop {
        let page = preview(after, 1)?;
        seen.extend(ids(&page));
        after = page["next_after"].clone();
        if after.is_null() {
            break;
        }
    }
    assert_eq!(seen, vec!["!a", "$schema", "b", "c"]);
    let create = open();
    let rejected=call(json!({"op":"save_delta","epoch":epoch,"workspace":create,"expected_version":0,"request_id":Uuid::new_v4(),"operations":[{"method":"post","client_id":"reserved","body":{"type":"Feature","id":"$schema","properties":{"id":"$schema"},"geometry":null}}]})).unwrap_err();
    assert_eq!(rejected.status, 422);
    assert_eq!(
        call(json!({"op":"draft_changes","epoch":epoch,"workspace":create}))?["version"],
        0
    );
    assert_eq!(call(json!({"op":"save_delta","epoch":epoch,"workspace":create,"expected_version":0,"request_id":Uuid::new_v4(),"operations":[{"method":"restore","id":"$schema","body":{}}]})).unwrap_err().status,422);
    let reserved = format!("{table}_reserved");
    db.batch_execute(&format!("CREATE TABLE public.{reserved}(id text PRIMARY KEY,geom geometry(Geometry,3857)); INSERT INTO public.{reserved}(id) VALUES('$schema');"))?;
    let other = Scope {
        dataset: reserved.clone(),
        ..scope
    };
    let host = TestHost {
        table: reserved.clone(),
        fail: false,
    };
    for invalid in [
        "$schema".to_owned(),
        String::new(),
        "x".repeat(257),
        "bad\u{85}".to_owned(),
    ] {
        db.execute(&format!("UPDATE public.{reserved} SET id=$1"), &[&invalid])?;
        assert_eq!(
            app.collaborate(&other, CollaborationCommand::Register, &host)
                .unwrap_err()
                .status,
            422
        );
    }
    assert_eq!(
        db.query_one(
            "SELECT count(*) FROM _geoledger_center.collab_datasets WHERE dataset=$1",
            &[&reserved]
        )?
        .get::<_, i64>(0),
        0
    );
    db.batch_execute(&format!(
        "DROP TABLE public.{table}; DROP TABLE public.{table}_marker; DROP TABLE public.{reserved}"
    ))?;
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn oversized_source_registration_rolls_back_and_can_retry() -> Result<(), Box<dyn std::error::Error>>
{
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let app = CenterApplication::new(dsn.clone());
    app.check_test_database()?;
    app.migrate()?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    let table = format!("collab_{}", Uuid::new_v4().simple());
    db.batch_execute(&format!("CREATE TABLE public.{table}(id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,label text,geom geometry(Geometry,3857)); INSERT INTO public.{table}(label) VALUES(repeat('x',4194304));"))?;
    let host = TestHost {
        table: table.clone(),
        fail: false,
    };
    let scope = Scope {
        subject: "alice".into(),
        tenant: "test".into(),
        project: "project".into(),
        dataset: table.clone(),
    };
    assert_eq!(
        app.collaborate(&scope, CollaborationCommand::Register, &host)
            .unwrap_err()
            .status,
        413
    );
    assert_eq!(
        db.query_one(
            "SELECT count(*) FROM _geoledger_center.collab_datasets WHERE dataset=$1",
            &[&table]
        )?
        .get::<_, i64>(0),
        0
    );
    db.batch_execute(&format!("UPDATE public.{table} SET label='small'"))?;
    assert_eq!(
        app.collaborate(&scope, CollaborationCommand::Register, &host)?["revision"],
        "0"
    );
    db.batch_execute(&format!("DROP TABLE public.{table}"))?;
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn geometry_choices_expire_and_rebase_retries_preserve_draft()
-> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let app = CenterApplication::new(dsn.clone());
    app.check_test_database()?;
    app.migrate()?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    let table = format!("collab_{}", Uuid::new_v4().simple());
    db.batch_execute(&format!("CREATE TABLE public.{table}(id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,a integer NOT NULL,geom geometry(Geometry,3857)); CREATE TABLE public.{table}_marker(revision bigint); INSERT INTO public.{table}_marker VALUES(0); INSERT INTO public.{table}(a,geom) VALUES(1,ST_SetSRID(ST_Point(0,0),3857)),(2,NULL);"))?;
    let host = TestHost {
        table: table.clone(),
        fail: false,
    };
    let scope = Scope {
        subject: "alice".into(),
        tenant: "test".into(),
        project: "project".into(),
        dataset: table.clone(),
    };
    let call = |v: Value| app.collaborate(&scope, serde_json::from_value(v).unwrap(), &host);
    let epoch = call(json!({"op":"register"}))?["epoch"].clone();
    let open = |base: &str| {
        call(json!({"op":"open_draft","epoch":epoch,"base_revision":base,"request_id":Uuid::new_v4()})).unwrap()["workspace"].clone()
    };
    let save = |w: &Value, operations: Value| {
        call(
            json!({"op":"save_delta","epoch":epoch,"workspace":w,"expected_version":0,"request_id":Uuid::new_v4(),"operations":operations}),
        )
    };
    let publish = |w: &Value, version: i64| {
        call(
            json!({"op":"publish","epoch":epoch,"workspace":w,"expected_version":version,"request_id":Uuid::new_v4()}),
        )
    };
    let a = open("0");
    let b = open("0");
    for (workspace, x) in [(&a, 1), (&b, 2)] {
        save(
            workspace,
            json!([{"method":"patch","id":"1","body":{"geometry":{"type":"Point","coordinates":[x,1]}}}]),
        )?;
    }
    publish(&a, 1)?;
    let preview = call(json!({"op":"preview","epoch":epoch,"workspace":b}))?;
    assert_eq!(preview["conflicts"][0]["fields"], json!(["/geometry"]));
    call(
        json!({"op":"resolve","epoch":epoch,"workspace":b,"expected_version":1,"expected_head":"1","resolutions":[{"id":"1","choice":"local"}]}),
    )?;
    let other = open("1");
    save(
        &other,
        json!([{"method":"patch","id":"2","body":{"properties":{"a":3}}}]),
    )?;
    publish(&other, 1)?;
    let rejected = publish(&b, 2).unwrap_err();
    assert_eq!(
        rejected.body["conflicts"][0]["fields"],
        json!(["stale_resolution"])
    );
    call(
        json!({"op":"resolve","epoch":epoch,"workspace":b,"expected_version":2,"expected_head":"2","resolutions":[{"id":"1","choice":"local"}]}),
    )?;
    let request = json!({"op":"rebase","epoch":epoch,"workspace":b,"expected_version":3,"expected_head":"2","request_id":Uuid::new_v4()});
    let rebased = call(request.clone())?;
    assert_eq!(call(request)?, rebased);
    assert_eq!(rebased["base_revision"], "2");
    let changes = call(json!({"op":"draft_changes","epoch":epoch,"workspace":b}))?;
    assert_eq!(changes["changes"].as_array().unwrap().len(), 1);
    assert_eq!(changes["changes"][0]["id"], "1");
    publish(&b, 4)?;
    let deleted = open("3");
    let changed = open("3");
    save(&deleted, json!([{"method":"delete","id":"1"}]))?;
    save(
        &changed,
        json!([{"method":"patch","id":"1","body":{"properties":{"a":8}}}]),
    )?;
    publish(&changed, 1)?;
    assert_eq!(
        publish(&deleted, 1).unwrap_err().body["conflicts"][0]["fields"],
        json!(["*"])
    );
    let mut denied = scope.clone();
    denied.tenant = "other".into();
    assert_eq!(
        app.collaborate(
            &denied,
            serde_json::from_value(json!({"op":"head"}))?,
            &host
        )
        .unwrap_err()
        .status,
        403
    );
    assert_eq!(
        call(json!({"op":"snapshot","epoch":Uuid::new_v4(),"revision":"0"}))
            .unwrap_err()
            .body["error"]["code"],
        "epoch_mismatch"
    );
    let keep = open("4");
    let drop = open("4");
    save(
        &keep,
        json!([{"method":"patch","id":"2","body":{"properties":{"a":10}}}]),
    )?;
    save(
        &drop,
        json!([{"method":"patch","schema":true,"body":{"drop":["a"]}}]),
    )?;
    publish(&drop, 1)?;
    let preview = call(json!({"op":"preview","epoch":epoch,"workspace":keep}))?;
    assert_eq!(preview["total"], 1);
    call(
        json!({"op":"resolve","epoch":epoch,"workspace":keep,"expected_version":1,"expected_head":"5","resolutions":[{"id":"$schema","choice":"local"}]}),
    )?;
    call(
        json!({"op":"rebase","epoch":epoch,"workspace":keep,"expected_version":2,"expected_head":"5","request_id":Uuid::new_v4()}),
    )?;
    publish(&keep, 3)?;
    let restored = db.query(&format!("SELECT a FROM public.{table} ORDER BY id"), &[])?;
    assert_eq!(restored[0].get::<_, i32>(0), 8);
    assert_eq!(restored[1].get::<_, i32>(0), 10);
    let replace = open("6");
    assert_eq!(save(&replace,json!([{"method":"patch","schema":true,"body":{"drop":["a"],"add":[{"name":"a","type":"text"}]}}])).unwrap_err().status,422);
    let exact_geometry: String = db
        .query_one(
            &format!("SELECT encode(ST_AsEWKB(geom),'hex') FROM public.{table} WHERE id=1"),
            &[],
        )?
        .get(0);
    let local = open("6");
    let remote = open("6");
    save(
        &local,
        json!([{"method":"patch","id":"1","body":{"properties":{"a":12}}}]),
    )?;
    save(&remote, json!([{"method":"delete","id":"1"}]))?;
    publish(&remote, 1)?;
    call(
        json!({"op":"resolve","epoch":epoch,"workspace":local,"expected_version":1,"expected_head":"7","resolutions":[{"id":"1","choice":"local"}]}),
    )?;
    call(
        json!({"op":"rebase","epoch":epoch,"workspace":local,"expected_version":2,"expected_head":"7","request_id":Uuid::new_v4()}),
    )?;
    let restored = call(json!({"op":"draft_changes","epoch":epoch,"workspace":local}))?;
    assert_eq!(restored["changes"][0]["restore"]["properties"]["a"], 8);
    let resumed = open("7");
    save(
        &resumed,
        json!([{"method":"restore","id":"1","body":{"properties":{"a":12}}}]),
    )?;
    publish(&resumed, 1)?;
    let row = db.query_one(
        &format!("SELECT a,encode(ST_AsEWKB(geom),'hex') FROM public.{table} WHERE id=1"),
        &[],
    )?;
    assert_eq!(row.get::<_, i32>(0), 12);
    assert_eq!(row.get::<_, String>(1), exact_geometry);
    db.batch_execute(&format!(
        "DROP TABLE public.{table}; DROP TABLE public.{table}_marker"
    ))?;
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn managed_table_merge_retry_native_geometry_and_host_rollback()
-> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let app = CenterApplication::new(dsn.clone());
    app.check_test_database()?;
    app.migrate()?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    let table = format!("collab_{}", Uuid::new_v4().simple());
    db.batch_execute(&format!("CREATE TABLE public.{table}(id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, a integer, b integer, exact numeric, geom geometry(Geometry,3857)); CREATE TABLE public.{table}_marker(revision bigint); INSERT INTO public.{table}_marker VALUES(0); INSERT INTO public.{table}(a,b,exact,geom) VALUES(1,1,12345678901234567890.123456789,ST_SetSRID(ST_MakePoint(123456.123456789,234567.987654321),3857)),(2,2,2,NULL);"))?;
    db.batch_execute(
        &(0..55)
            .map(|i| format!("ALTER TABLE public.{table} ADD COLUMN extra_{i} text;"))
            .collect::<String>(),
    )?;
    let host = TestHost {
        table: table.clone(),
        fail: false,
    };
    let scope = Scope {
        subject: "alice".into(),
        tenant: "test".into(),
        project: "project".into(),
        dataset: table.clone(),
    };
    let call = |s: &Scope, v: Value| app.collaborate(s, serde_json::from_value(v).unwrap(), &host);
    let original: String = db
        .query_one(
            &format!("SELECT encode(ST_AsEWKB(geom),'hex') FROM public.{table} WHERE id=1"),
            &[],
        )?
        .get(0);
    let head = call(&scope, json!({"op":"register"}))?;
    let epoch = head["epoch"].clone();
    assert_eq!(head["revision"], "0");
    let initial = call(
        &scope,
        json!({"op":"snapshot","epoch":epoch,"revision":"0"}),
    )?;
    assert_eq!(
        initial["features"][0]["properties"]["exact"],
        "12345678901234567890.123456789"
    );
    assert_eq!(initial["features"][0]["properties"]["id"], "1");
    let mut bob = scope.clone();
    bob.subject = "bob".into();
    let open = |s: &Scope| {
        call(s,json!({"op":"open_draft","epoch":epoch,"base_revision":"0","request_id":Uuid::new_v4()})).unwrap()["workspace"].clone()
    };
    let a = open(&scope);
    let b = open(&bob);
    let save = |s: &Scope, w: &Value, value: Value| {
        call(
            s,
            json!({"op":"save_delta","epoch":epoch,"workspace":w,"expected_version":0,"request_id":Uuid::new_v4(),"operations":[{"method":"patch","id":"1","body":{"properties":value}}]}),
        )
    };
    save(&scope, &a, json!({"a":3}))?;
    save(&bob, &b, json!({"b":4}))?;
    let publish = |s: &Scope, w: &Value, version: i64| {
        call(
            s,
            json!({"op":"publish","epoch":epoch,"workspace":w,"expected_version":version,"request_id":Uuid::new_v4()}),
        )
    };
    publish(&scope, &a, 1)?;
    let request = json!({"op":"publish","epoch":epoch,"workspace":b,"expected_version":1,"request_id":Uuid::new_v4()});
    let result = call(&bob, request.clone())?;
    assert_eq!(call(&bob, request.clone())?, result);
    assert_eq!(result["revision"], "2");
    let row = db.query_one(
        &format!(
            "SELECT a,b,exact::text,encode(ST_AsEWKB(geom),'hex') FROM public.{table} WHERE id=1"
        ),
        &[],
    )?;
    assert_eq!((row.get::<_, i32>(0), row.get::<_, i32>(1)), (3, 4));
    assert_eq!(row.get::<_, String>(2), "12345678901234567890.123456789");
    assert_eq!(row.get::<_, String>(3), original);
    let c = open(&scope);
    save(&scope, &c, json!({"a":5}))?;
    assert_eq!(publish(&scope, &c, 1).unwrap_err().status, 409);
    let preview = call(&scope, json!({"op":"preview","epoch":epoch,"workspace":c}))?;
    assert_eq!(preview["total"], 1);
    assert!(
        preview["conflicts"][0]["fields"]
            .as_array()
            .unwrap()
            .contains(&json!("/properties/a"))
    );
    call(
        &scope,
        json!({"op":"resolve","epoch":epoch,"workspace":c,"expected_version":1,"expected_head":"2","resolutions":[{"id":"1","choice":"fields","fields":{"/properties/a":"local"}}]}),
    )?;
    let fail_host = TestHost {
        table: table.clone(),
        fail: true,
    };
    let failing: CollaborationCommand = serde_json::from_value(
        json!({"op":"publish","epoch":epoch,"workspace":c,"expected_version":2,"request_id":Uuid::new_v4()}),
    )?;
    assert_eq!(
        app.collaborate(&scope, failing, &fail_host)
            .unwrap_err()
            .status,
        422
    );
    assert_eq!(call(&scope, json!({"op":"head"}))?["revision"], "2");
    assert_eq!(
        db.query_one(&format!("SELECT revision FROM public.{table}_marker"), &[])?
            .get::<_, i64>(0),
        2
    );
    publish(&scope, &c, 2)?;
    let row = db.query_one(
        &format!(
            "SELECT a,b,exact::text,encode(ST_AsEWKB(geom),'hex') FROM public.{table} WHERE id=1"
        ),
        &[],
    )?;
    assert_eq!(row.get::<_, i32>(0), 5);
    assert_eq!(row.get::<_, i32>(1), 4);
    assert_eq!(row.get::<_, String>(2), "12345678901234567890.123456789");
    assert_eq!(row.get::<_, String>(3), original);
    let page = call(
        &scope,
        json!({"op":"snapshot","epoch":epoch,"revision":"0","limit":1}),
    )?;
    assert_eq!(page["features"][0]["properties"]["a"], 1);
    assert!(call(&scope,json!({"op":"snapshot","epoch":epoch,"revision":"3","limit":1,"after":page["next_after"]})).is_err());
    let old = call(
        &scope,
        json!({"op":"snapshot","epoch":epoch,"revision":"0","after":page["next_after"]}),
    )?;
    assert_eq!(old["features"][0]["id"], "2");
    assert!(
        call(
            &bob,
            json!({"op":"draft_changes","epoch":epoch,"workspace":c})
        )
        .is_err()
    );
    db.batch_execute(&format!(
        "DROP TABLE public.{table}; DROP TABLE public.{table}_marker"
    ))?;
    Ok(())
}

#[test]
#[ignore = "requires isolated GL_TEST_DATABASE_URL database geoledger_test with PostGIS"]
fn schema_conflicts_and_large_chunked_drafts_have_stable_generated_ids()
-> Result<(), Box<dyn std::error::Error>> {
    let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
    let app = CenterApplication::new(dsn.clone()).with_timeout(std::time::Duration::from_secs(180));
    app.check_test_database()?;
    app.migrate()?;
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls)?;
    let table = format!("collab_{}", Uuid::new_v4().simple());
    db.batch_execute(&format!("CREATE TABLE public.{table}(id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,a integer,b integer DEFAULT 7,geom geometry(Geometry,3857)); CREATE TABLE public.{table}_marker(revision bigint); INSERT INTO public.{table}_marker VALUES(0); INSERT INTO public.{table}(a,b) VALUES(1,1);"))?;
    let host = TestHost {
        table: table.clone(),
        fail: false,
    };
    let scope = Scope {
        subject: "alice".into(),
        tenant: "test".into(),
        project: "project".into(),
        dataset: table.clone(),
    };
    let call = |v: Value| app.collaborate(&scope, serde_json::from_value(v).unwrap(), &host);
    let epoch = call(json!({"op":"register"}))?["epoch"].clone();
    let open = |base: &str| {
        call(json!({"op":"open_draft","epoch":epoch,"base_revision":base,"request_id":Uuid::new_v4()})).unwrap()["workspace"].clone()
    };
    let save = |w: &Value, version: i64, operations: Value| {
        call(
            json!({"op":"save_delta","epoch":epoch,"workspace":w,"expected_version":version,"request_id":Uuid::new_v4(),"operations":operations}),
        )
    };
    let publish = |w: &Value, version: i64| {
        call(
            json!({"op":"publish","epoch":epoch,"workspace":w,"expected_version":version,"request_id":Uuid::new_v4()}),
        )
    };
    let drop = open("0");
    let edit = open("0");
    save(
        &drop,
        0,
        json!([{"method":"patch","schema":true,"body":{"drop":["a"]}}]),
    )?;
    save(
        &edit,
        0,
        json!([{"method":"patch","id":"1","body":{"properties":{"a":2}}}]),
    )?;
    publish(&edit, 1)?;
    let preview = call(json!({"op":"preview","epoch":epoch,"workspace":drop}))?;
    assert_eq!(preview["conflicts"][0]["id"], "$schema");
    assert_eq!(publish(&drop, 1).unwrap_err().status, 409);
    let resolution = json!({"op":"resolve","epoch":epoch,"workspace":drop,"expected_version":1,"expected_head":"1","request_id":Uuid::new_v4(),"resolutions":[{"id":"$schema","choice":"local"}]});
    let resolved = call(resolution.clone())?;
    assert_eq!(call(resolution)?, resolved);
    publish(&drop, 2)?;
    let snap = call(json!({"op":"snapshot","epoch":epoch,"revision":"2"}))?;
    assert!(snap["features"][0]["properties"].get("a").is_none());
    assert_eq!(
        call(json!({"op":"snapshot","epoch":epoch,"revision":"0"}))?["features"][0]["properties"]["a"],
        1
    );
    let add = open("2");
    save(
        &add,
        0,
        json!([{"method":"patch","schema":true,"body":{"add":[{"name":"title","type":"text"}]}},{"method":"patch","id":"1","body":{"properties":{"title":"saved"}}}]),
    )?;
    publish(&add, 1)?;
    assert_eq!(
        db.query_one(&format!("SELECT title FROM public.{table} WHERE id=1"), &[])?
            .get::<_, String>(0),
        "saved"
    );
    let large = open("3");
    let geometry = json!({"type":"LineString","coordinates":(0..2000).map(|i|json!([i as f64/10000.0,1.0])).collect::<Vec<_>>()});
    assert!(geometry.to_string().len() > 16384);
    let first = json!({"op":"save_delta","epoch":epoch,"workspace":large,"expected_version":0,"request_id":Uuid::new_v4(),"operations":[{"method":"post","client_id":"large-geometry","body":{"type":"Feature","properties":{"b":9,"title":"large"},"geometry":geometry}}]});
    let saved = call(first.clone())?;
    assert_eq!(saved, call(first.clone())?);
    let mut wrong = first;
    wrong["operations"][0]["body"]["properties"]["b"] = json!(8);
    assert_eq!(call(wrong).unwrap_err().status, 409);
    let mut mapping = saved["ids"].as_object().unwrap().clone();
    for batch in 0..6 {
        let operations=(0..200).map(|i|json!({"method":"post","client_id":format!("new-{batch}-{i}"),"body":{"type":"Feature","properties":{"b":i},"geometry":null}})).collect::<Vec<_>>();
        let result = save(&large, batch + 1, json!(operations))?;
        mapping.extend(result["ids"].as_object().unwrap().clone());
    }
    assert_eq!(mapping.len(), 1201);
    let started = std::time::Instant::now();
    let published = publish(&large, 7)?;
    assert_eq!(published["changes"], 1201);
    assert_eq!(published["ids"], json!(mapping));
    let mut count = 0;
    let mut after = Value::Null;
    let mut bytes = 0;
    loop {
        let page = call(
            json!({"op":"changes","epoch":epoch,"from":"3","to":"4","after":after,"limit":137}),
        )?;
        count += page["changes"].as_array().unwrap().len();
        bytes += page.to_string().len();
        if page["done"] == true {
            break;
        }
        after = page["next_after"].clone();
    }
    assert_eq!(count, 1201);
    assert_eq!(
        db.query_one(&format!("SELECT count(*) FROM public.{table}"), &[])?
            .get::<_, i64>(0),
        1202
    );
    eprintln!(
        "collaboration fixture: 1201 changes, geometry {} bytes, delta {} bytes, publish+read {:?}",
        geometry.to_string().len(),
        bytes,
        started.elapsed()
    );
    let del = open("4");
    save(&del, 0, json!([{"method":"delete","id":"1"}]))?;
    publish(&del, 1)?;
    assert!(
        call(json!({"op":"changes","epoch":epoch,"from":"4","to":"5"}))?["changes"][0]["feature"]
            .is_null()
    );
    let noop = open("5");
    let unchanged = publish(&noop, 0)?;
    assert_eq!(unchanged["revision"], "5");
    assert_eq!(unchanged["status"], "unchanged");
    let defaults = open("5");
    let saved = save(
        &defaults,
        0,
        json!([{"method":"post","client_id":"default","body":{"type":"Feature","properties":{},"geometry":null}},{"method":"post","client_id":"null","body":{"type":"Feature","properties":{"b":null},"geometry":null}}]),
    )?;
    publish(&defaults, 1)?;
    let default_id = saved["ids"]["default"].as_str().unwrap();
    let null_id = saved["ids"]["null"].as_str().unwrap();
    assert_eq!(
        db.query_one(
            &format!("SELECT b FROM public.{table} WHERE id::text=$1"),
            &[&default_id]
        )?
        .get::<_, Option<i32>>(0),
        Some(7)
    );
    assert_eq!(
        db.query_one(
            &format!("SELECT b FROM public.{table} WHERE id::text=$1"),
            &[&null_id]
        )?
        .get::<_, Option<i32>>(0),
        None
    );
    db.batch_execute(&format!(
        "DROP TABLE public.{table}; DROP TABLE public.{table}_marker"
    ))?;
    Ok(())
}
