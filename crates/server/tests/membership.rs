//! Membership and project lifecycle through the Rust SDK over real gRPC, plus HTTP.
use geoledger_client::{Client, Page};
use geoledger_server::{
    Application, Authentication, Authenticator, Policy, Service, Storage, Tokens,
};
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
// Public, test-only credentials for an ephemeral in-process server.
const ALICE: &str = "test-only-membership-alice-credential-0123456789";
const BOB: &str = "test-only-membership-bob-credential-0123456789ab";
const ROOT: &str = "test-only-membership-root-credential-0123456789a";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn members_archive_delete_and_admin_over_grpc_and_http() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app =
        Application::new(Storage::Sqlite(dir.path().join("members.sqlite3"))).with_policy(Policy {
            admins: ["root".to_owned()].into(),
            admin_only_project_creation: false,
            max_owned_projects: Some(2),
        });
    app.migrate()?;
    let tokens = Tokens::from_json(
        format!(r#"[{{"subject":"alice","token_sha256":"{}"}},{{"subject":"bob","token_sha256":"{}"}},{{"subject":"root","token_sha256":"{}"}}]"#, geoledger_server::sha256_hex(ALICE), geoledger_server::sha256_hex(BOB), geoledger_server::sha256_hex(ROOT))
            .as_bytes(),
    )?;
    let service = Service::new(app, Authentication::new(Authenticator::Tokens(tokens)));
    let grpc = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let (grpc_addr, http_addr) = (grpc.local_addr()?, http.local_addr()?);
    let router = geoledger_server::router(service.clone());
    tokio::spawn(async move { axum::serve(http, router).await });
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(grpc))
            .await
    });
    let endpoint = format!("http://{grpc_addr}");
    let alice = Client::connect(&endpoint, ALICE).await?;
    let bob = Client::connect(&endpoint, BOB).await?;
    let root = Client::connect(&endpoint, ROOT).await?;

    let project = alice.create_project("lifecycle").await?;
    assert_eq!(project.state, "active");
    alice.set_member(&project.id, "bob", "editor").await?;
    let members = bob.members(&project.id, Page::default()).await?;
    let subjects: Vec<_> = members
        .iter()
        .map(|m| (m.subject.as_str(), m.role.as_str()))
        .collect();
    assert_eq!(subjects, [("alice", "owner"), ("bob", "editor")]);
    // HTTP exposes the same operation.
    let response = reqwest_like_post(
        http_addr,
        "list_members",
        ALICE,
        &format!(r#"{{"project":"{}"}}"#, project.id),
    )
    .await?;
    assert!(response.contains(r#""subject":"bob""#), "{response}");

    let archived = alice.archive_project(&project.id, true).await?;
    assert_eq!(archived.state, "archived");
    let blocked = bob
        .create_workspace(&project.id)
        .await
        .err()
        .ok_or("archived write accepted")?;
    assert_eq!(blocked.code, "conflict");
    assert_eq!(bob.project(&project.id).await?.state, "archived");
    alice.archive_project(&project.id, false).await?;

    bob.remove_member(&project.id, "bob").await?;
    assert_eq!(
        bob.project(&project.id).await.err().ok_or("left")?.code,
        "not_found"
    );
    let last_owner = alice
        .remove_member(&project.id, "alice")
        .await
        .err()
        .ok_or("last owner removed")?;
    assert_eq!(last_owner.code, "conflict");

    // Quota: alice owns one project and may create exactly one more.
    alice.create_project("second").await?;
    let quota = alice
        .create_project("third")
        .await
        .err()
        .ok_or("quota ignored")?;
    assert_eq!(quota.code, "permission_denied");

    // A platform administrator can recover a project and hand it over.
    assert_eq!(root.project(&project.id).await?.role, "admin");
    root.set_member(&project.id, "bob", "owner").await?;
    let wrong = bob
        .delete_project(&project.id, "not the name")
        .await
        .err()
        .ok_or("wrong name")?;
    assert_eq!(wrong.code, "invalid_argument");
    bob.delete_project(&project.id, "lifecycle").await?;
    for client in [&alice, &bob, &root] {
        assert_eq!(
            client
                .project(&project.id)
                .await
                .err()
                .ok_or("deleted")?
                .code,
            "not_found"
        );
    }
    assert!(
        alice
            .projects(Page::default())
            .await?
            .iter()
            .all(|p| p.id != project.id)
    );
    Ok(())
}

async fn reqwest_like_post(
    addr: std::net::SocketAddr,
    op: &str,
    token: &str,
    body: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let request = format!(
        "POST /api/v1/{op} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    Ok(response)
}
