//! Exercise error contracts over real HTTP/2 with a bounded metadata budget.
mod common;
use geoledger_rpc::v1 as pb;
use geoledger_server::{Application, Service, Storage};
use prost::Message;
use serde_json::json;
use tonic::{Request, transport::Endpoint};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn authenticated<T>(body: T) -> Result<Request<T>, Box<dyn std::error::Error + Send + Sync>> {
    let mut request = Request::new(body);
    // Public, test-only credential for this ephemeral in-process server.
    request.metadata_mut().insert(
        "authorization",
        "Bearer glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse()?,
    );
    Ok(request)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_conflicts_fit_trailers_and_keep_paginated_details() -> TestResult {
    let dir = common::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("errors.sqlite3")));
    app.migrate()?;
    common::seed(
        &dir.path().join("errors.sqlite3"),
        &[("review", common::KEY, false)],
    )?;
    let project = app.execute(
        &common::user_id("review"),
        "create_project",
        json!({"name":"errors"}),
    )?["project"]
        .as_str()
        .ok_or("project")?
        .to_owned();
    let dataset = app.execute(
        &common::user_id("review"),
        "create_dataset",
        json!({"geometry_type":"point","project":project,"name":"features"}),
    )?["dataset"]
        .as_str()
        .ok_or("dataset")?
        .to_owned();
    let mut workspaces = Vec::new();
    for value in ["a", "b"] {
        let workspace = app.execute(
            &common::user_id("review"),
            "create_workspace",
            json!({"project":project}),
        )?["workspace"]
            .as_str()
            .ok_or("workspace")?
            .to_owned();
        let edits: Vec<_> = (0..20)
            .map(|i| {
                let id = format!("feature-{i:02}");
                json!({"dataset":dataset,"feature_id":id,"feature":{"type":"Feature","id":id,"geometry":null,"properties":{"content":value.repeat(7000)}}})
            })
            .collect();
        app.execute(&common::user_id("review"), "save", json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"edits":edits}))?;
        workspaces.push(workspace);
    }
    app.execute(&common::user_id("review"), "publish", json!({"project":project,"workspace":workspaces[0],"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"first"}))?;

    let service = Service::new(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let channel = Endpoint::from_shared(endpoint)?
        .http2_max_header_list_size(8192)
        .connect()
        .await?;
    let mut client = pb::geo_ledger_client::GeoLedgerClient::new(channel);
    let error = client
        .publish(authenticated(pb::PublishRequest {
            project: project.clone(),
            workspace: workspaces[1].clone(),
            expected_workspace_version: 1,
            request_id: uuid::Uuid::new_v4().to_string(),
            message: "conflicting publication".into(),
        })?)
        .await
        .err()
        .ok_or("publication must conflict")?;
    assert_eq!(error.code(), tonic::Code::Aborted);
    assert!(error.details().len() < 4096);
    let envelope = pb::RpcStatus::decode(error.details())?;
    let detail = pb::ErrorDetail::decode(
        envelope
            .details
            .first()
            .ok_or("error detail")?
            .value
            .as_slice(),
    )?;
    assert_eq!(detail.code, "conflict");
    assert!(uuid::Uuid::parse_str(&detail.request_id).is_ok());
    let conflicts = detail.conflicts.ok_or("conflict summary")?;
    assert_eq!(
        (conflicts.head, conflicts.version, conflicts.total),
        (1, 1, 20)
    );
    assert!(conflicts.truncated);
    assert!(conflicts.conflicts.is_empty());
    assert_eq!(conflicts.next_after, None);

    let page = client
        .conflicts(authenticated(pb::DiffRequest {
            project: project.clone(),
            workspace: workspaces[1].clone(),
            after: String::new(),
            limit: Some(5),
        })?)
        .await?
        .into_inner();
    assert_eq!(page.total, 20);
    assert_eq!(page.conflicts.len(), 5);
    assert!(page.truncated);
    assert!(
        page.conflicts[0]
            .draft
            .as_ref()
            .ok_or("draft")?
            .geojson
            .contains(&"b".repeat(7000))
    );
    let next = client
        .conflicts(authenticated(pb::DiffRequest {
            project: project.clone(),
            workspace: workspaces[1].clone(),
            after: page.next_after.ok_or("cursor")?,
            limit: Some(5),
        })?)
        .await?
        .into_inner();
    assert_eq!(next.conflicts.len(), 5);
    assert_ne!(page.conflicts[0].feature_id, next.conflicts[0].feature_id);
    assert_eq!(
        client
            .get_project(authenticated(pb::ProjectRequest {
                project: project.clone()
            })?)
            .await?
            .into_inner()
            .head,
        1
    );
    let duplicate = client
        .create_dataset(authenticated(pb::DatasetRequest {
            postgis_table: None,
            coordinate_dimension: 2,
            crs: "EPSG:4326".into(),
            geometry_type: "point".into(),
            project,
            name: "features".into(),
        })?)
        .await
        .err()
        .ok_or("duplicate")?;
    assert_eq!(duplicate.code(), tonic::Code::Aborted);
    servers.abort_all();
    Ok(())
}

// These opt-in fixtures require psql, but add no PostgreSQL dependency to the server.
// Every destructive statement follows a live database-name check.
struct PostgisFixture {
    dsn: String,
}
impl PostgisFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let dsn = std::env::var("GL_TEST_DATABASE_URL")?;
        if Self::sql(&dsn, "SELECT current_database()")? != "geoledger_test" {
            return Err("requires dedicated geoledger_test database".into());
        }
        let fixture = Self { dsn };
        fixture.reset()?;
        Ok(fixture)
    }
    fn sql(dsn: &str, sql: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let output = std::process::Command::new("psql")
            .args([
                "-X",
                "-q",
                "-A",
                "-t",
                "-v",
                "ON_ERROR_STOP=1",
                "--dbname",
                dsn,
                "--command",
                sql,
            ])
            .env("PGCONNECT_TIMEOUT", "5")
            .env(
                "PGOPTIONS",
                "-c statement_timeout=10000 -c lock_timeout=5000",
            )
            .output()?;
        if !output.status.success() {
            // Do not include connection arguments or driver diagnostics in test output.
            return Err("PostGIS fixture SQL failed".into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
    fn query(&self, sql: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Self::sql(&self.dsn, sql)
    }
    fn reset(&self) -> TestResult {
        if self.query("SELECT current_database()")? != "geoledger_test" {
            return Err("live database is not geoledger_test".into());
        }
        self.query("DROP SCHEMA IF EXISTS business CASCADE; DROP SCHEMA public CASCADE; CREATE SCHEMA public; CREATE EXTENSION IF NOT EXISTS postgis")?;
        Ok(())
    }
    async fn service(&self) -> Result<Service, Box<dyn std::error::Error + Send + Sync>> {
        let dsn = self.dsn.clone();
        let app = tokio::task::spawn_blocking(move || {
            let app = Application::new(Storage::Postgis(dsn));
            app.migrate()?;
            Ok::<_, geoledger_engine::Error>(app)
        })
        .await??;
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(common::KEY.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        self.query(&format!(
            "INSERT INTO gl_users(id,username,display_name,status,is_admin,must_change_password,created_at) VALUES('{id}','recovery','recovery','active',true,false,'2026-01-01T00:00:00Z');
             INSERT INTO gl_passwords(user_id,password_hash) VALUES('{id}','unused-password-fixture');
             INSERT INTO gl_api_keys(id,user_id,digest,name,key_prefix,created_at,last_used_at) VALUES('{key_id}','{id}','{digest}','test','{prefix}','2026-01-01T00:00:00Z','2099-01-01T00:00:00Z')",
            id=common::user_id("recovery"), key_id=uuid::Uuid::new_v4(), prefix=&common::KEY[..12],
        ))?;
        Ok(Service::new(app))
    }
    fn snapshot(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        ["gl_projects", "gl_workspaces", "gl_workspace_changes", "gl_workspace_sizes",
         "gl_commits", "gl_commit_changes", "gl_history", "gl_schema_history",
         "gl_workspace_schemas", "gl_idempotency", "gl_audit_events", "gl_source_state",
         "gl_source_changes", "gl_source_versions", "business.points"]
            .into_iter()
            .map(|table| self.query(&format!("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text FROM {table} t")))
            .collect()
    }
}
impl Drop for PostgisFixture {
    fn drop(&mut self) {
        let _ = self.reset();
    }
}
fn source_feature(id: &str, label: &str) -> serde_json::Value {
    json!({"type":"Feature","id":id,"properties":{"label":label,"n":18446744073709551615u64},"geometry":null})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires psql and isolated geoledger_test PostGIS database"]
async fn postgis_publication_recovery_and_legacy_transport_are_atomic() -> TestResult {
    use axum::{body::Body, http::Request as HttpRequest};
    use geoledger_client::{Client, Edit, PostgisTable, Publication};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let db = PostgisFixture::new()?;
    db.query("CREATE SCHEMA business; CREATE TABLE business.points(id bigint PRIMARY KEY,label text NOT NULL CHECK(label<>'reject-secret'),n numeric(20,0),geom geometry(Point,4326)); INSERT INTO business.points VALUES(1,'original',18446744073709551615,NULL)")?;
    let service = db.service().await?;
    let router = geoledger_server::router(service.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let mut servers = tokio::task::JoinSet::new();
    servers.spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
    });
    let client = Client::connect(&endpoint, common::KEY).await?;
    let project = client.create_project("source recovery").await?;
    let dataset = client
        .register_table(
            &project.id,
            "points",
            &PostgisTable {
                schema: "business".into(),
                table: "points".into(),
                id_column: "id".into(),
                geometry_column: "geom".into(),
            },
        )
        .await?;
    let mut workspace = client.create_workspace(&project.id).await?;
    workspace
        .save_batch(&[
            Edit {
                dataset: dataset.id.clone(),
                feature_id: "1".into(),
                feature: Some(source_feature("1", "edited")),
            },
            Edit {
                dataset: dataset.id.clone(),
                feature_id: "2".into(),
                feature: Some(source_feature("2", "reject-secret")),
            },
        ])
        .await?;
    let before = db.snapshot()?;
    let error = workspace
        .publish("original")
        .await
        .err()
        .ok_or("CHECK accepted")?;
    assert_eq!(error.code, "publication_attempt_rejected");
    assert!(!error.uncertain);
    assert!(error.request_id.is_some());
    assert!(!error.to_string().contains("reject-secret"));
    assert!(workspace.pending_publication().is_none());
    assert_eq!(workspace.info().version, 1);
    assert_eq!(db.snapshot()?, before);

    let intent = Publication {
        project: project.id.clone(),
        workspace: workspace.id().into(),
        expected_workspace_version: 1,
        request_id: uuid::Uuid::new_v4().to_string(),
        message: "low-level rejection".into(),
    };
    let error = client
        .publish(&intent)
        .await
        .err()
        .ok_or("low-level CHECK accepted")?;
    assert_eq!(error.code, "publication_attempt_rejected");
    assert!(
        error.uncertain,
        "low-level consumers lack first-attempt context"
    );
    let mut raw = pb::geo_ledger_client::GeoLedgerClient::connect(endpoint).await?;
    let status = raw
        .publish(authenticated(pb::PublishRequest {
            project: intent.project.clone(),
            workspace: intent.workspace.clone(),
            expected_workspace_version: intent.expected_workspace_version,
            request_id: intent.request_id.clone(),
            message: intent.message.clone(),
        })?)
        .await
        .err()
        .ok_or("raw CHECK accepted")?;
    assert_eq!(
        status.code(),
        tonic::Code::Unavailable,
        "older consumers must retain unknown outcomes"
    );
    let envelope = pb::RpcStatus::decode(status.details())?;
    let detail =
        pb::ErrorDetail::decode(envelope.details.first().ok_or("detail")?.value.as_slice())?;
    assert_eq!(detail.code, "publication_attempt_rejected");
    let response = router
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/api/v1/publish")
                .header("authorization", format!("Bearer {}", common::KEY))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&intent)?))?,
        )
        .await?;
    assert_eq!(response.status().as_u16(), 503);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await?.to_bytes())?;
    assert_eq!(body["error"]["code"], "publication_attempt_rejected");
    assert_eq!(db.snapshot()?, before);

    // The same high-level handle can correct its saved draft and publish once.
    workspace
        .save(&dataset.id, source_feature("2", "inserted"))
        .await?;
    let result = workspace.publish("corrected").await?;
    assert_eq!(result.revision, 2);
    assert_eq!(workspace.publish("corrected").await?, result);
    assert_eq!(
        client
            .publish(workspace.pending_publication().ok_or("successful intent")?)
            .await?,
        result
    );
    assert_eq!(db.query("SELECT count(*) FROM gl_idempotency")?, "1");
    assert_eq!(db.query("SELECT count(*) FROM gl_commits")?, "2");
    assert_eq!(
        db.query("SELECT count(*) FROM gl_audit_events WHERE action='publish'")?,
        "1"
    );
    assert_eq!(
        db.query("SELECT label||':'||n::text FROM business.points ORDER BY id")?,
        "edited:18446744073709551615\ninserted:18446744073709551615"
    );
    servers.abort_all();
    while servers.join_next().await.is_some() {}
    Ok(())
}

// A transport queue retains the complete first request before it enters the engine.
// Returning Unavailable does not discard that request; the test controls its later
// execution. This is deliberately not an engine-worker or rollback-ack hook.
type DelayedGate = (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<Result<u16, String>>,
);
#[derive(Clone)]
struct DelayedPublication {
    inner: geoledger_server::Grpc,
    gate: std::sync::Arc<tokio::sync::Mutex<Option<DelayedGate>>>,
    payloads: std::sync::Arc<tokio::sync::Mutex<Vec<Vec<u8>>>>,
}
impl tonic::server::NamedService for DelayedPublication {
    const NAME: &'static str = <geoledger_server::Grpc as tonic::server::NamedService>::NAME;
}
impl tower::Service<axum::http::Request<tonic::body::Body>> for DelayedPublication {
    type Response = axum::http::Response<tonic::body::Body>;
    type Error = std::convert::Infallible;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;
    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        tower::Service::poll_ready(&mut self.inner, cx)
    }
    fn call(&mut self, request: axum::http::Request<tonic::body::Body>) -> Self::Future {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let inner = self.inner.clone();
        let gate = self.gate.clone();
        let payloads = self.payloads.clone();
        Box::pin(async move {
            if request.uri().path() != "/geoledger.v1.GeoLedger/Publish" {
                return inner.oneshot(request).await;
            }
            let (parts, body) = request.into_parts();
            let bytes = match body.collect().await {
                Ok(body) => body.to_bytes(),
                Err(error) => return Ok(error.into_http()),
            };
            payloads.lock().await.push(bytes.to_vec());
            let request = axum::http::Request::from_parts(
                parts,
                tonic::body::Body::new(axum::body::Body::from(bytes)),
            );
            if let Some((release, completed)) = gate.lock().await.take() {
                // Detach the accepted request from the response waiter intentionally.
                drop(tokio::spawn(async move {
                    let result = async {
                        release
                            .await
                            .map_err(|_| "delayed request was not released".to_owned())?;
                        let response = match inner.oneshot(request).await {
                            Ok(response) => response,
                            Err(never) => match never {},
                        };
                        let header_status = tonic::Status::from_header_map(response.headers())
                            .map(|s| s.code() as u16);
                        let body = response
                            .into_body()
                            .collect()
                            .await
                            .map_err(|e| e.to_string())?;
                        let trailer_status = body
                            .trailers()
                            .and_then(tonic::Status::from_header_map)
                            .map(|s| s.code() as u16);
                        Ok(header_status.or(trailer_status).unwrap_or(0))
                    }
                    .await;
                    let _ = completed.send(result);
                }));
                Ok(tonic::Status::unavailable(
                    "response lost while the original request remained queued",
                )
                .into_http())
            } else {
                inner.oneshot(request).await
            }
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires psql and isolated geoledger_test PostGIS database"]
async fn postgis_late_original_attempt_cannot_be_settled_by_retry_rejection() -> TestResult {
    use geoledger_client::{Client, PostgisTable};
    use std::{sync::Arc, time::Duration};
    // Both outcomes matter: a late original can commit after external conditions
    // change, or become stale after a separately authorized handle edits the draft.
    for advance_version in [false, true] {
        let db = PostgisFixture::new()?;
        db.query("CREATE SCHEMA business; CREATE TABLE business.publication_gate(allowed boolean NOT NULL); INSERT INTO business.publication_gate VALUES(false); CREATE FUNCTION business.publication_allowed() RETURNS boolean LANGUAGE sql STABLE AS $$ SELECT allowed FROM business.publication_gate $$; CREATE TABLE business.points(id bigint PRIMARY KEY,label text NOT NULL CHECK(label<>'late' OR business.publication_allowed()),n numeric(20,0),geom geometry(Point,4326)); INSERT INTO business.points VALUES(1,'original',18446744073709551615,NULL)")?;
        let service = db.service().await?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let (release, wait) = tokio::sync::oneshot::channel();
        let (complete, completed) = tokio::sync::oneshot::channel();
        let payloads = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let delayed = DelayedPublication {
            inner: geoledger_server::grpc(service),
            gate: Arc::new(tokio::sync::Mutex::new(Some((wait, complete)))),
            payloads: payloads.clone(),
        };
        let mut servers = tokio::task::JoinSet::new();
        servers.spawn(async move {
            tonic::transport::Server::builder()
                .add_service(delayed)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
        });
        let client = Client::connect(endpoint, common::KEY).await?;
        let project = client.create_project("late publication").await?;
        let dataset = client
            .register_table(
                &project.id,
                "points",
                &PostgisTable {
                    schema: "business".into(),
                    table: "points".into(),
                    id_column: "id".into(),
                    geometry_column: "geom".into(),
                },
            )
            .await?;
        let mut workspace = client.create_workspace(&project.id).await?;
        workspace
            .save(&dataset.id, source_feature("1", "late"))
            .await?;
        let unknown = workspace
            .publish("original")
            .await
            .err()
            .ok_or("expected response loss")?;
        assert!(unknown.uncertain);
        let original = workspace
            .pending_publication()
            .ok_or("missing unknown intent")?
            .clone();
        let before = db.snapshot()?;
        let rejection = workspace
            .publish("original")
            .await
            .err()
            .ok_or("expected source rejection")?;
        assert_eq!(rejection.code, "publication_attempt_rejected");
        assert!(rejection.uncertain);
        assert_eq!(workspace.pending_publication(), Some(&original));
        assert_eq!(db.snapshot()?, before);
        assert_eq!(
            workspace
                .save(&dataset.id, source_feature("1", "changed"))
                .await
                .err()
                .ok_or("unknown outcome allowed edits")?
                .code,
            "invalid_argument"
        );
        assert_eq!(workspace.pending_publication(), Some(&original));
        if advance_version {
            let mut other = client.workspace(&project.id, workspace.id()).await?;
            other
                .save(&dataset.id, source_feature("1", "corrected"))
                .await?;
        }
        db.query("UPDATE business.publication_gate SET allowed=true")?;
        release
            .send(())
            .map_err(|_| "delayed request disappeared")?;
        let old_status = tokio::time::timeout(Duration::from_secs(10), completed)
            .await??
            .map_err(std::io::Error::other)?;
        if advance_version {
            assert_eq!(old_status, tonic::Code::Aborted as u16);
            assert_eq!(db.query("SELECT count(*) FROM gl_idempotency")?, "0");
            assert_eq!(db.query("SELECT head FROM gl_projects")?, "1");
            let stale = workspace
                .publish("original")
                .await
                .err()
                .ok_or("stale request accepted")?;
            assert_eq!(stale.code, "conflict");
            assert!(!stale.uncertain);
            assert!(workspace.pending_publication().is_none());
            workspace.refresh().await?;
            assert_eq!(workspace.info().version, 2);
            workspace.publish("corrected after stale").await?;
        } else {
            assert_eq!(old_status, tonic::Code::Ok as u16);
            let receipt = workspace.publish("original").await?;
            assert_eq!(receipt.revision, 2);
            assert_eq!(workspace.publish("original").await?, receipt);
            assert_eq!(workspace.pending_publication(), Some(&original));
            let persisted: serde_json::Value = serde_json::from_str(&db.query(&format!(
                "SELECT payload FROM gl_idempotency WHERE request_id='{}'",
                original.request_id
            ))?)?;
            assert_eq!(persisted, serde_json::to_value(&original)?);
        }
        let requests = payloads.lock().await;
        assert!(requests.len() >= 3);
        assert_eq!(requests[0], requests[1]);
        assert_eq!(requests[0], requests[2]);
        assert_eq!(db.query("SELECT head FROM gl_projects")?, "2");
        assert_eq!(db.query("SELECT count(*) FROM gl_commits")?, "2");
        assert_eq!(db.query("SELECT count(*) FROM gl_idempotency")?, "1");
        assert_eq!(
            db.query("SELECT count(*) FROM gl_audit_events WHERE action='publish'")?,
            "1"
        );
        servers.abort_all();
        while servers.join_next().await.is_some() {}
    }
    Ok(())
}
