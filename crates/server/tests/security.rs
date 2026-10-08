//! Transport and credential lifecycle against the real server binary: TLS on both
//! listeners, mutual TLS, hashed tokens with expiry/revocation and hot reload.
use geoledger_client::{Client, ConnectOptions};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair};
use std::{
    net::{SocketAddr, TcpListener},
    path::Path,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;
type BoxError = Box<dyn std::error::Error + Send + Sync>;

struct Server {
    child: Child,
    http: SocketAddr,
    grpc: SocketAddr,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn ports() -> Result<(SocketAddr, SocketAddr), BoxError> {
    let h = TcpListener::bind("127.0.0.1:0")?;
    let g = TcpListener::bind("127.0.0.1:0")?;
    Ok((h.local_addr()?, g.local_addr()?))
}
fn start(dir: &Path, extra: &[(&str, String)]) -> Result<Server, BoxError> {
    let (http, grpc) = ports()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_geoledger-server"));
    command
        .args(["--data-dir", dir.to_str().ok_or("path")?])
        .args(["--http", &http.to_string(), "--grpc", &grpc.to_string()])
        .env("GL_RELOAD_INTERVAL_SECS", "1");
    for key in [
        "GL_STORAGE",
        "GL_DATABASE_URL",
        "GL_TOKEN_FILE",
        "GL_JWKS_FILE",
        "GL_JWKS_URL",
        "GL_TLS_CERT",
        "GL_TLS_KEY",
        "GL_TLS_CLIENT_CA",
    ] {
        command.env_remove(key);
    }
    for (k, v) in extra {
        command.env(k, v);
    }
    let child = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Server { child, http, grpc })
}
struct Pki {
    ca_pem: String,
    server_cert: String,
    server_key: String,
    client_cert: String,
    client_key: String,
}
fn pki() -> Result<Pki, BoxError> {
    let ca_key = KeyPair::generate()?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "geoledger test-only CA");
    let ca = ca_params.self_signed(&ca_key)?;
    let server_key = KeyPair::generate()?;
    let server = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])?.signed_by(
        &server_key,
        &ca,
        &ca_key,
    )?;
    let client_key = KeyPair::generate()?;
    let mut client_params = CertificateParams::new(Vec::<String>::new())?;
    client_params
        .distinguished_name
        .push(DnType::CommonName, "operator");
    let client = client_params.signed_by(&client_key, &ca, &ca_key)?;
    Ok(Pki {
        ca_pem: ca.pem(),
        server_cert: server.pem(),
        server_key: server_key.serialize_pem(),
        client_cert: client.pem(),
        client_key: client_key.serialize_pem(),
    })
}
fn write_tokens(path: &Path, entries: serde_json::Value) -> Result<(), BoxError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(&entries)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
async fn wait_for<F, Fut>(mut f: F) -> Result<(), BoxError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if f().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err("condition not reached".into())
}
async fn https_get(
    addr: SocketAddr,
    pki: &Pki,
    client_auth: bool,
    path: &str,
) -> Result<String, BoxError> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from_pem_slice(pki.ca_pem.as_bytes())?)?;
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots);
    let config = if client_auth {
        builder.with_client_auth_cert(
            vec![CertificateDer::from_pem_slice(pki.client_cert.as_bytes())?],
            PrivateKeyDer::from_pem_slice(pki.client_key.as_bytes())?,
        )?
    } else {
        builder.with_no_client_auth()
    };
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let mut tls = connector
        .connect(ServerName::try_from("localhost")?, tcp)
        .await?;
    tls.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await?;
    let mut out = String::new();
    tls.read_to_string(&mut out).await?;
    Ok(out)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_listeners_and_mutual_tls() -> TestResult {
    let dir = tempfile::tempdir()?;
    let pki = pki()?;
    for (name, body) in [
        ("ca.pem", &pki.ca_pem),
        ("server.pem", &pki.server_cert),
        ("server.key", &pki.server_key),
    ] {
        std::fs::write(dir.path().join(name), body)?;
    }
    let data = dir.path().join("data");
    let server = start(
        &data,
        &[
            (
                "GL_TLS_CERT",
                dir.path().join("server.pem").display().to_string(),
            ),
            (
                "GL_TLS_KEY",
                dir.path().join("server.key").display().to_string(),
            ),
        ],
    )?;
    let credentials = data.join("admin-credentials.json");
    let server_tokens = data.join("tokens.json");
    wait_for(|| {
        let ready = credentials.exists() && server_tokens.exists();
        async move { ready }
    })
    .await?;
    let tokens: serde_json::Value = serde_json::from_slice(&std::fs::read(&credentials)?)?;
    let token = tokens[0]["token"].as_str().ok_or("token")?.to_owned();
    // The server file stores only a digest.
    let server_file = std::fs::read_to_string(data.join("tokens.json"))?;
    assert!(!server_file.contains(&token));
    assert!(server_file.contains(&geoledger_server::sha256_hex(&token)));
    let options = ConnectOptions {
        ca_pem: Some(pki.ca_pem.clone().into_bytes()),
        ..Default::default()
    };
    let endpoint = format!("https://localhost:{}", server.grpc.port());
    let mut connected = None;
    for _ in 0..100 {
        if let Ok(c) = Client::connect_with(&endpoint, &token, options.clone()).await
            && c.info().await.is_ok()
        {
            connected = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let client = connected.ok_or("TLS gRPC unavailable")?;
    assert_eq!(client.info().await?.backend, "sqlite");
    // Plaintext gRPC against the TLS listener fails instead of downgrading.
    let plain = Client::connect(format!("http://127.0.0.1:{}", server.grpc.port()), &token).await;
    assert!(match plain {
        Ok(c) => c.info().await.is_err(),
        Err(_) => true,
    });
    let health = https_get(server.http, &pki, false, "/health").await?;
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    drop(server);

    // Mutual TLS: the same server now requires a client certificate signed by the CA.
    let mtls = start(
        &data,
        &[
            (
                "GL_TLS_CERT",
                dir.path().join("server.pem").display().to_string(),
            ),
            (
                "GL_TLS_KEY",
                dir.path().join("server.key").display().to_string(),
            ),
            (
                "GL_TLS_CLIENT_CA",
                dir.path().join("ca.pem").display().to_string(),
            ),
        ],
    )?;
    let mut ok = String::new();
    for _ in 0..100 {
        if let Ok(r) = https_get(mtls.http, &pki, true, "/health").await {
            ok = r;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(https_get(mtls.http, &pki, false, "/health").await.is_err());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_rotation_expiry_and_revocation_without_restart() -> TestResult {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("server-tokens.json");
    let first = "f".repeat(64);
    let second = "s".repeat(64);
    let expired = "e".repeat(64);
    write_tokens(
        &file,
        serde_json::json!([
            {"subject":"alice","token_sha256":geoledger_server::sha256_hex(&first)},
            {"subject":"bob","token_sha256":geoledger_server::sha256_hex(&expired),"expires_at":"2000-01-01T00:00:00Z"},
        ]),
    )?;
    let server = start(
        &dir.path().join("data"),
        &[("GL_TOKEN_FILE", file.display().to_string())],
    )?;
    let endpoint = format!("http://{}", server.grpc);
    let works = |token: String| {
        let endpoint = endpoint.clone();
        async move {
            match Client::connect(&endpoint, &token).await {
                Ok(c) => c.info().await.is_ok(),
                Err(_) => false,
            }
        }
    };
    wait_for(|| works(first.clone())).await?;
    assert!(!works(expired.clone()).await, "expired token");
    assert!(!works(second.clone()).await);
    // Rotate: add the second token, disable the first.
    write_tokens(
        &file,
        serde_json::json!([
            {"subject":"alice","token_sha256":geoledger_server::sha256_hex(&first),"disabled":true},
            {"subject":"alice","token_sha256":geoledger_server::sha256_hex(&second),"label":"2026-10"},
        ]),
    )?;
    wait_for(|| works(second.clone())).await?;
    assert!(!works(first.clone()).await, "revoked token");
    // An invalid file never replaces the active configuration.
    std::fs::write(&file, b"not json")?;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(works(second.clone()).await);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwks_url_is_refreshed_and_keeps_last_good_keys() -> TestResult {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture: serde_json::Value = serde_json::from_slice(include_bytes!("fixtures/jwks.json"))?;
    let mut renamed = fixture.clone();
    renamed["keys"][0]["kid"] = "previous".into();
    let current = Arc::new(std::sync::Mutex::new(serde_json::to_vec(&renamed)?));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let issuer_addr = listener.local_addr()?;
    let served = current.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = served.lock().map(|b| b.clone()).unwrap_or_default();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
        }
    });
    let dir = tempfile::tempdir()?;
    let server = start(
        &dir.path().join("data"),
        &[
            ("GL_JWKS_URL", format!("http://{issuer_addr}/jwks.json")),
            ("GL_JWKS_REFRESH_SECS", "1".into()),
            ("GL_JWT_ISSUER", "https://issuer.example".into()),
            ("GL_JWT_AUDIENCE", "geoledger".into()),
        ],
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-only".into());
    // Publicly committed fixture key; exclusively for tests.
    let token = encode(
        &header,
        &serde_json::json!({"iss":"https://issuer.example","aud":"geoledger","sub":"alice","exp":now+600}),
        &EncodingKey::from_rsa_pem(include_bytes!("fixtures/jwt-test-only.pem"))?,
    )?;
    let endpoint = format!("http://{}", server.grpc);
    let works = |token: String| {
        let endpoint = endpoint.clone();
        async move {
            match Client::connect(&endpoint, &token).await {
                Ok(c) => c.info().await.is_ok(),
                Err(_) => false,
            }
        }
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!works(token.clone()).await, "kid not yet published");
    *current.lock().map_err(|_| "lock")? = serde_json::to_vec(&fixture)?;
    wait_for(|| works(token.clone())).await?;
    // A broken document keeps the last good key set.
    *current.lock().map_err(|_| "lock")? = b"{}".to_vec();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(works(token.clone()).await);
    Ok(())
}
