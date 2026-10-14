//! Transport and credential lifecycle against the real server binary: TLS on both
//! listeners and mutual TLS with database API keys.
mod common;
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
    std::fs::create_dir_all(dir)?;
    common::database(&dir.join("geoledger.sqlite3"))?;
    let (http, grpc) = ports()?;
    let mut command = Command::new(env!("CARGO_BIN_EXE_geoledger-server"));
    command
        .args(["--data-dir", dir.to_str().ok_or("path")?])
        .args(["--http", &http.to_string(), "--grpc", &grpc.to_string()])
        .env("GL_RELOAD_INTERVAL_SECS", "1");
    for key in [
        "GL_STORAGE",
        "GL_DATABASE_URL",
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
        .stderr(Stdio::inherit())
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
    let dir = common::tempdir()?;
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
    let token = common::KEY.to_owned();
    let options = ConnectOptions {
        ca_pem: Some(pki.ca_pem.clone().into_bytes()),
        ..Default::default()
    };
    let endpoint = format!("https://localhost:{}", server.grpc.port());
    let mut connected = None;
    let mut last_error = String::new();
    for _ in 0..100 {
        match Client::connect_with(&endpoint, &token, options.clone()).await {
            Ok(client) => match client.info().await {
                Ok(_) => {
                    connected = Some(client);
                    break;
                }
                Err(error) => last_error = error.to_string(),
            },
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let client = connected.ok_or_else(|| format!("TLS gRPC unavailable: {last_error}"))?;
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
