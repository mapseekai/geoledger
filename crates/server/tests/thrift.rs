#![allow(clippy::unwrap_used)]
use spatial_version::Application;
use spatial_version_server::{Service, thrift, thrift_proto::*};
use std::{io, time::Duration};
use volo::net::{
    conn::Conn,
    incoming::{DefaultIncoming, Incoming, MakeIncoming},
};
use volo_thrift::{MaybeException, codec::default::DefaultMakeCodec};

#[derive(Debug)]
struct StoppableIncoming {
    inner: DefaultIncoming,
    stop: tokio::sync::oneshot::Receiver<()>,
}
impl MakeIncoming for StoppableIncoming {
    type Incoming = Self;
    async fn make_incoming(self) -> io::Result<Self> {
        Ok(self)
    }
}
impl Incoming for StoppableIncoming {
    async fn accept(&mut self) -> io::Result<Option<Conn>> {
        tokio::select! {
            _ = &mut self.stop => Ok(None),
            result = self.inner.accept() => result,
        }
    }
}
fn reply<E>(result: MaybeException<JsonReply, E>) -> serde_json::Value {
    match result {
        MaybeException::Ok(reply) => serde_json::from_str(&reply.json).unwrap(),
        MaybeException::Exception(_) => panic!("unexpected Thrift application exception"),
    }
}

#[tokio::test]
async fn thrift_roundtrip_auth_errors_and_shared_application() {
    let directory = tempfile::tempdir().unwrap();
    let app = Application::new(directory.path());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(thrift::run(
        Service::new(
            app.clone(),
            Some("thrift-token-long-enough-for-tests".into()),
        ),
        StoppableIncoming {
            inner: listener.into(),
            stop: rx,
        },
    ));
    let client = SpatialVersionClientBuilder::new("spatial-version")
        .address(address)
        .make_codec(DefaultMakeCodec::framed())
        .rpc_timeout(Some(Duration::from_secs(10)))
        .build();
    let auth = Some("Bearer thrift-token-long-enough-for-tests".into());
    let unauthorized = client
        .status(StatusRequest { limit: 1 }, None)
        .await
        .unwrap();
    match unauthorized {
        MaybeException::Exception(SpatialVersionStatusException::Error(e)) => {
            assert_eq!(e.code, "unauthenticated")
        }
        _ => panic!("missing authorization accepted"),
    }
    // Rejected calls must not create repository state.
    assert!(!directory.path().join(".spatial-version").exists());
    reply(
        client
            .execute(
                ExecuteRequest {
                    command_json: r#"{"op":"init","author":"thrift-test"}"#.into(),
                },
                auth.clone(),
            )
            .await
            .unwrap(),
    );
    reply(
        client
            .branch(
                BranchRequest {
                    name: "thrift-draft".into(),
                    from: "".into(),
                },
                auth.clone(),
            )
            .await
            .unwrap(),
    );
    let status = reply(
        client
            .status(StatusRequest { limit: 0 }, auth.clone())
            .await
            .unwrap(),
    );
    assert_eq!(status["branch"], "main");
    assert_eq!(status["clean"], true);
    let log = reply(
        client
            .log(
                LogRequest {
                    reference: "thrift-draft".into(),
                    limit: 0,
                },
                auth.clone(),
            )
            .await
            .unwrap(),
    );
    assert_eq!(log["commits"].as_array().unwrap().len(), 1);
    let direct = app
        .execute(serde_json::from_str(r#"{"op":"status"}"#).unwrap())
        .unwrap();
    assert_eq!(status, direct);
    let invalid = client
        .status(StatusRequest { limit: -1 }, auth.clone())
        .await
        .unwrap();
    assert!(
        matches!(invalid, MaybeException::Exception(SpatialVersionStatusException::Error(e)) if e.code == "invalid_argument")
    );
    let invalid = client
        .execute(
            ExecuteRequest {
                command_json: "{".into(),
            },
            auth.clone(),
        )
        .await
        .unwrap();
    assert!(
        matches!(invalid, MaybeException::Exception(SpatialVersionExecuteException::Error(e)) if e.code == "invalid_argument")
    );
    let missing = client
        .log(
            LogRequest {
                reference: "missing".into(),
                limit: 1,
            },
            auth.clone(),
        )
        .await
        .unwrap();
    assert!(
        matches!(missing, MaybeException::Exception(SpatialVersionLogException::Error(e)) if e.code == "not_found")
    );
    let large = client
        .execute(
            ExecuteRequest {
                command_json: " "
                    .repeat(spatial_version_server::MAX_REQUEST_BYTES + 1)
                    .into(),
            },
            auth,
        )
        .await
        .unwrap();
    assert!(
        matches!(large, MaybeException::Exception(SpatialVersionExecuteException::Error(e)) if e.code == "invalid_argument")
    );
    drop(client);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn thrift_refuses_non_loopback_without_token_before_binding() {
    let directory = tempfile::tempdir().unwrap();
    let result = thrift::serve(
        Application::new(directory.path()),
        "0.0.0.0:0".parse().unwrap(),
        None,
    )
    .await;
    assert!(matches!(
        result,
        Err(spatial_version_core::Error::Invalid(_))
    ));
}

#[test]
#[ignore = "requires disposable SV_TEST_DATABASE_URL"]
fn thrift_postgis_schema_changes_roundtrip() {
    let dsn = std::env::var("SV_TEST_DATABASE_URL").unwrap();
    let mut db = postgres::Client::connect(&dsn, postgres::NoTls).unwrap();
    let name: String = db
        .query_one("SELECT current_database()", &[])
        .unwrap()
        .get(0);
    assert_eq!(name, "spatial_version_test", "refusing a non-test database");
    let schema = format!("svthrift_{}", uuid::Uuid::new_v4().simple());
    db.batch_execute(&format!("CREATE SCHEMA {schema}; CREATE TABLE {schema}.roads (id bigint PRIMARY KEY, name text); INSERT INTO {schema}.roads VALUES (1, 'road')")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let app = Application::new(directory.path()).with_provider(std::sync::Arc::new(
        spatial_version_postgis::PostgisProvider::new(dsn),
    ));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(thrift::run(Service::new(app, None), StoppableIncoming { inner: listener.into(), stop: rx }));
        let client = SpatialVersionClientBuilder::new("spatial-version")
            .address(address).make_codec(DefaultMakeCodec::framed())
            .rpc_timeout(Some(Duration::from_secs(30))).build();
        use serde_json::json;
        for command in [
            json!({"op":"init"}),
            json!({"op":"import","dataset":"roads","schema":schema,"table":"roads"}),
            json!({"op":"alter_schema","dataset":"roads","change":{"action":"add","name":"note","data_type":"text"}}),
            json!({"op":"alter_schema","dataset":"roads","change":{"action":"rename","name":"note","new_name":"memo"}}),
            json!({"op":"alter_schema","dataset":"roads","change":{"action":"alter_type","name":"memo","data_type":"varchar(200)"}}),
        ] {
            reply(client.execute(ExecuteRequest { command_json: command.to_string().into() }, None).await.unwrap());
        }
        let schema_reply = reply(client.execute(ExecuteRequest { command_json: json!({"op":"schema","dataset":"roads"}).to_string().into() }, None).await.unwrap());
        let fields = schema_reply["schema"]["fields"].as_array().unwrap();
        let memo = fields.iter().find(|f| f["name"] == "memo").unwrap();
        assert_eq!(memo["metadata"]["postgres.type"], "character varying(200)");
        assert!(!fields.iter().any(|f| f["name"] == "note"));
        reply(client.execute(ExecuteRequest { command_json: json!({"op":"alter_schema","dataset":"roads","change":{"action":"drop","name":"memo","discard":true}}).to_string().into() }, None).await.unwrap());
        let status = reply(client.status(StatusRequest { limit: 1 }, None).await.unwrap());
        assert_eq!(status["clean"], true);
        let log = reply(client.log(LogRequest { reference: "HEAD".into(), limit: 20 }, None).await.unwrap());
        assert_eq!(log["commits"].as_array().unwrap().len(), 6);
        reply(client.execute(ExecuteRequest { command_json: json!({"op":"fsck"}).to_string().into() }, None).await.unwrap());
        drop(client);
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), server).await.unwrap().unwrap().unwrap();
    });
    let columns: Vec<String> = db.query("SELECT column_name FROM information_schema.columns WHERE table_schema=$1 AND table_name='roads' ORDER BY ordinal_position", &[&schema]).unwrap().into_iter().map(|r| r.get(0)).collect();
    assert_eq!(columns, vec!["id", "name"]);
    let value: String = db
        .query_one(&format!("SELECT name FROM {schema}.roads WHERE id=1"), &[])
        .unwrap()
        .get(0);
    assert_eq!(value, "road");
    db.batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .unwrap();
}
