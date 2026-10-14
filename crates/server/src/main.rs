use clap::{Parser, Subcommand, ValueEnum};
use geoledger_server::{
    Application, Limits, Rate, Service, Storage, StorageOptions,
    tls::{Tls, TlsFiles, TlsListener},
};
use std::{
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
type BoxError = Box<dyn std::error::Error + Send + Sync>;
mod data;
#[derive(Parser)]
#[command(
    name = "geoledger-server",
    version,
    about = "Spatial version control server: SQLite by default, optional PostGIS"
)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, env = "GL_STORAGE", default_value = "sqlite")]
    storage: Backend,
    #[arg(long, env = "GL_DATA_DIR", default_value = "./geoledger-data")]
    data_dir: PathBuf,
    #[arg(long, env = "GL_DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    /// Permit sslmode=disable to a non-loopback PostgreSQL host (trusted private network only).
    #[arg(long, env = "GL_DATABASE_ALLOW_PLAINTEXT", default_value_t = false)]
    database_allow_plaintext: bool,
    #[arg(long, env = "GL_HTTP_LISTEN", default_value = "127.0.0.1:7881")]
    http: SocketAddr,
    #[arg(long, env = "GL_GRPC_LISTEN", default_value = "127.0.0.1:7882")]
    grpc: SocketAddr,
    /// PEM certificate chain; enables TLS on both listeners together with --tls-key.
    #[arg(long, env = "GL_TLS_CERT")]
    tls_cert: Option<PathBuf>,
    /// PEM private key (PKCS#8, PKCS#1 or SEC1).
    #[arg(long, env = "GL_TLS_KEY")]
    tls_key: Option<PathBuf>,
    /// PEM CA bundle; when set, clients must present a certificate it signed (mTLS).
    #[arg(long, env = "GL_TLS_CLIENT_CA")]
    tls_client_ca: Option<PathBuf>,
    #[arg(long, env = "GL_ADMIN_USERNAME", default_value = "admin")]
    admin_username: String,
    // Passwords are accepted only from the environment, never command-line arguments.
    /// How often TLS files are checked for changes (SIGHUP reloads immediately).
    #[arg(long, env = "GL_RELOAD_INTERVAL_SECS", default_value_t = 10)]
    reload_interval_secs: u64,
    /// Optional plaintext listener serving only /health and /ready (for container probes).
    #[arg(long, env = "GL_HEALTH_LISTEN")]
    health_listen: Option<SocketAddr>,
    /// Concurrent application operations (HTTP + gRPC); excess calls get 429.
    #[arg(long, env = "GL_MAX_CONCURRENCY", default_value_t = 20)]
    max_concurrency: usize,
    /// Upper bound for every operation, including client gRPC deadlines.
    #[arg(long, env = "GL_REQUEST_TIMEOUT_SECS", default_value_t = 30)]
    request_timeout_secs: u64,
    /// Maximum encoded request (unary or complete streamed upload).
    #[arg(long, env = "GL_MAX_REQUEST_BYTES", default_value_t = 64 * 1024 * 1024)]
    max_request_bytes: usize,
    #[arg(long, env = "GL_MAX_RESPONSE_BYTES", default_value_t = 64 * 1024 * 1024)]
    max_response_bytes: usize,
    #[arg(long, env = "GL_REQUEST_MEMORY_BYTES", default_value_t = 256 * 1024 * 1024)]
    request_memory_bytes: usize,
    #[arg(long, env = "GL_RESPONSE_MEMORY_BYTES", default_value_t = 256 * 1024 * 1024)]
    response_memory_bytes: usize,
    /// Maximum PostgreSQL connections.
    #[arg(long, env = "GL_DB_POOL_SIZE", default_value_t = 20)]
    db_pool_size: usize,
    #[arg(long, env = "GL_DB_STATEMENT_TIMEOUT_SECS", default_value_t = 30)]
    db_statement_timeout_secs: u64,
    #[arg(long, env = "GL_DB_LOCK_TIMEOUT_SECS", default_value_t = 10)]
    db_lock_timeout_secs: u64,
    /// Sustained requests per second per authenticated subject (0 disables).
    #[arg(long, env = "GL_RATE_LIMIT_SUBJECT_RPS", default_value_t = 0.0)]
    rate_limit_subject_rps: f64,
    #[arg(long, env = "GL_RATE_LIMIT_SUBJECT_BURST", default_value_t = 50.0)]
    rate_limit_subject_burst: f64,
    /// Sustained requests per second per client IP, checked before authentication (0 disables).
    #[arg(long, env = "GL_RATE_LIMIT_IP_RPS", default_value_t = 0.0)]
    rate_limit_ip_rps: f64,
    #[arg(long, env = "GL_RATE_LIMIT_IP_BURST", default_value_t = 100.0)]
    rate_limit_ip_burst: f64,
    /// Take the client IP from the right-most X-Forwarded-For entry (only behind a trusted gateway).
    #[arg(long, env = "GL_TRUST_FORWARDED_FOR", default_value_t = false, action = clap::ArgAction::Set)]
    trust_forwarded_for: bool,
    /// Seconds to keep serving while /ready reports 503 before closing listeners.
    #[arg(long, env = "GL_SHUTDOWN_DRAIN_SECS", default_value_t = 0)]
    shutdown_drain_secs: u64,
    /// Maximum seconds to wait for in-flight requests after listeners close.
    #[arg(long, env = "GL_SHUTDOWN_TIMEOUT_SECS", default_value_t = 30)]
    shutdown_timeout_secs: u64,
    /// Who may create projects.
    #[arg(long, env = "GL_PROJECT_CREATION", default_value = "anyone")]
    project_creation: ProjectCreation,
    /// Maximum projects a non-administrator may own (0 = unlimited).
    #[arg(long, env = "GL_MAX_PROJECTS_PER_SUBJECT", default_value_t = 0)]
    max_projects_per_subject: u32,
    /// Deadline for export, import, verify and backup commands.
    #[arg(long, env = "GL_DATA_TIMEOUT_SECS", default_value_t = 3600)]
    data_timeout_secs: u64,
}
#[derive(Clone, Copy, ValueEnum)]
enum ProjectCreation {
    Anyone,
    Admins,
}
#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Sqlite,
    Postgis,
}
#[derive(Subcommand)]
enum Command {
    /// Write a portable logical export (JSON lines with per-table digests and a
    /// checksum) of the configured database. Safe while the server runs.
    Export {
        /// New file (mode 0600, never overwritten) or `-` for stdout.
        #[arg(long)]
        output: PathBuf,
    },
    /// Load an export into the configured database, which must be new or empty.
    /// All-or-nothing: a damaged file leaves the database unchanged.
    Import {
        /// Export file or `-` for stdin.
        #[arg(long)]
        input: PathBuf,
    },
    /// Validate an export file and compare it with the configured database
    /// (exit 4 on mismatch).
    Verify {
        #[arg(long)]
        input: PathBuf,
        /// Only validate the file's structure, digests and checksum.
        #[arg(long)]
        file_only: bool,
    },
    /// Online consistent SQLite backup into a new file, then verify it.
    /// PostgreSQL deployments use pg_dump/WAL archiving or `export`.
    Backup {
        #[arg(long)]
        output: PathBuf,
    },
    /// Install a verified SQLite backup as the database of --data-dir. Refuses
    /// to replace an existing database; stop the server first.
    Restore {
        #[arg(long)]
        input: PathBuf,
    },
    /// Exit 0 when the local server reports ready (for container health checks).
    /// Uses GL_HEALTH_LISTEN when set, otherwise the HTTP listener.
    Probe {
        /// Explicit URL to probe instead of the configured listener.
        #[arg(long)]
        url: Option<String>,
    },
}
#[tokio::main]
async fn main() {
    let args = Args::parse();
    let logs = tracing_subscriber::fmt().json().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "geoledger_server=info,geoledger_engine=info".into()),
    );
    // Operator commands keep stdout for their own output (e.g. `export --output -`).
    if args.command.is_some() {
        logs.with_writer(std::io::stderr).init();
    } else {
        logs.init();
    }
    // Exit explicitly: a forced shutdown must not wait for abandoned blocking work.
    let code = match run(args).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("geoledger-server: {e}");
            1
        }
    };
    std::process::exit(code);
}
fn application(args: &Args, storage: Storage) -> Application {
    Application::with_options(
        storage,
        StorageOptions {
            pool_size: args.db_pool_size.max(1),
            statement_timeout: Duration::from_secs(args.db_statement_timeout_secs.max(1)),
            lock_timeout: Duration::from_secs(args.db_lock_timeout_secs.max(1)),
        },
    )
    .with_timeout(Duration::from_secs(600))
    .with_policy(geoledger_server::Policy {
        admin_only_project_creation: matches!(args.project_creation, ProjectCreation::Admins),
        max_owned_projects: (args.max_projects_per_subject > 0)
            .then_some(args.max_projects_per_subject),
    })
}
fn loopback(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, addr.port()))
        }
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, addr.port()))
        }
        _ => addr,
    }
}
async fn probe(args: &Args, url: Option<String>) -> Result<(), BoxError> {
    let url = url.unwrap_or_else(|| match args.health_listen {
        Some(addr) => format!("http://{}/ready", loopback(addr)),
        None if args.tls_cert.is_some() => format!("https://{}/ready", loopback(args.http)),
        None => format!("http://{}/ready", loopback(args.http)),
    });
    // The probe targets this host's own listener; certificate identity is checked by real clients.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .danger_accept_invalid_certs(true)
        .build()?;
    let response = client.get(&url).send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("{url} returned {}", response.status()).into())
    }
}
async fn run(args: Args) -> Result<(), BoxError> {
    match args.command {
        Some(Command::Probe { ref url }) => return probe(&args, url.clone()).await,
        Some(
            Command::Export { .. }
            | Command::Import { .. }
            | Command::Verify { .. }
            | Command::Backup { .. }
            | Command::Restore { .. },
        )
        | None => {}
    }
    if matches!(args.storage, Backend::Sqlite) && args.database_url.is_some() {
        return Err(
            "GL_DATABASE_URL requires --storage postgis; refusing to silently use SQLite".into(),
        );
    }
    if args.tls_cert.is_some() != args.tls_key.is_some() {
        return Err("GL_TLS_CERT and GL_TLS_KEY must be configured together".into());
    }
    if args.tls_client_ca.is_some() && args.tls_cert.is_none() {
        return Err("GL_TLS_CLIENT_CA requires GL_TLS_CERT and GL_TLS_KEY".into());
    }
    std::fs::create_dir_all(&args.data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&args.data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let storage = match args.storage {
        Backend::Sqlite => Storage::Sqlite(args.data_dir.join("geoledger.sqlite3")),
        Backend::Postgis => {
            let dsn = args
                .database_url
                .clone()
                .ok_or("PostGIS requires GL_DATABASE_URL")?;
            let summary = geoledger_engine::postgres_tls_summary(&dsn)?;
            if summary.mode == geoledger_engine::SslMode::Disable
                && !summary.loopback_only
                && !args.database_allow_plaintext
            {
                return Err("sslmode=disable to a non-loopback PostgreSQL host requires GL_DATABASE_ALLOW_PLAINTEXT=true".into());
            }
            tracing::info!(sslmode = summary.mode.as_str(), "PostgreSQL TLS policy");
            Storage::Postgis(dsn)
        }
    };
    let data_app = || {
        application(&args, storage.clone())
            .with_timeout(Duration::from_secs(args.data_timeout_secs.max(1)))
    };
    match &args.command {
        Some(Command::Export { output }) => {
            return data::export(data_app(), output.clone()).await;
        }
        Some(Command::Import { input }) => return data::import(data_app(), input.clone()).await,
        Some(Command::Verify { input, file_only }) => {
            return data::verify(data_app(), input.clone(), *file_only).await;
        }
        Some(Command::Backup { output }) => {
            return data::backup(data_app(), output.clone()).await;
        }
        Some(Command::Restore { input }) => {
            let Storage::Sqlite(target) = &storage else {
                return Err("restore installs SQLite backups; restore PostgreSQL with pg_restore or `import`".into());
            };
            return data::restore(
                input.clone(),
                target.clone(),
                Duration::from_secs(args.data_timeout_secs.max(1)),
            )
            .await;
        }
        _ => {}
    }
    let tls = match (&args.tls_cert, &args.tls_key) {
        (Some(cert), Some(key)) => Some(Tls::load(TlsFiles {
            cert: cert.clone(),
            key: key.clone(),
            client_ca: args.tls_client_ca.clone(),
        })?),
        _ => None,
    };
    let app = application(&args, storage);
    let bootstrap = app.clone();
    tokio::task::spawn_blocking(move || bootstrap.migrate()).await??;
    let initial = app.clone();
    let username = args.admin_username.clone();
    let data_dir = args.data_dir.clone();
    let password = std::env::var("GL_ADMIN_PASSWORD").ok();
    tokio::task::spawn_blocking(move || {
        initialize_admin(&initial, &data_dir, &username, password.as_deref())
    })
    .await??;
    let backend = app.backend();
    let limits = Limits {
        max_concurrency: args.max_concurrency.max(1),
        request_timeout: Duration::from_secs(args.request_timeout_secs.max(1)),
        per_subject: Rate::new(args.rate_limit_subject_rps, args.rate_limit_subject_burst),
        per_ip: Rate::new(args.rate_limit_ip_rps, args.rate_limit_ip_burst),
        trust_forwarded_for: args.trust_forwarded_for,
        max_request_bytes: args.max_request_bytes,
        max_response_bytes: args.max_response_bytes,
        request_memory_bytes: args.request_memory_bytes,
        response_memory_bytes: args.response_memory_bytes,
    };
    let service = Service::new(app).with_limits(limits);
    // Bind both before reporting readiness. A failed listener leaves no half-started service.
    let http = tokio::net::TcpListener::bind(args.http).await?;
    let grpc = tokio::net::TcpListener::bind(args.grpc).await?;
    let health = match args.health_listen {
        Some(addr) => Some(tokio::net::TcpListener::bind(addr).await?),
        None => None,
    };
    let http_addr = http.local_addr()?;
    let grpc_addr = grpc.local_addr()?;
    if tls.is_none() && !(http_addr.ip().is_loopback() && grpc_addr.ip().is_loopback()) {
        tracing::warn!(
            "listening on a non-loopback address without TLS; terminate TLS at a trusted gateway or set GL_TLS_CERT/GL_TLS_KEY"
        );
    }
    tokio::spawn(reload_loop(
        tls.clone(),
        Duration::from_secs(args.reload_interval_secs.max(1)),
    ));
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_serving::<geoledger_rpc::v1::geo_ledger_server::GeoLedgerServer<Service>>()
        .await;
    tracing::info!(http=%http_addr,grpc=%grpc_addr,%backend,tls=tls.is_some(),mtls=args.tls_client_ca.is_some(),max_concurrency=service.limits().max_concurrency,"GeoLedger ready");
    let (stop, rx) = tokio::sync::watch::channel(false);
    let mut http_stop = rx.clone();
    let mut grpc_stop = rx.clone();
    if let Some(listener) = health {
        let mut health_stop = rx;
        let router = geoledger_server::health_router(service.clone());
        tokio::spawn(async move {
            let shutdown = async move {
                let _ = health_stop.changed().await;
            };
            if let Err(error) = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
            {
                tracing::warn!(%error, "health listener stopped");
            }
        });
    }
    let draining = service.clone();
    let router = geoledger_server::router(service.clone())
        .into_make_service_with_connect_info::<geoledger_server::tls::PeerAddr>();
    let http_tls = tls.clone();
    let mut h = tokio::spawn(async move {
        let shutdown = async move {
            let _ = http_stop.changed().await;
        };
        match http_tls {
            Some(tls) => {
                axum::serve(TlsListener::new(http, tls)?, router)
                    .with_graceful_shutdown(shutdown)
                    .await
            }
            None => {
                axum::serve(http, router)
                    .with_graceful_shutdown(shutdown)
                    .await
            }
        }
    });
    let mut g = tokio::spawn(async move {
        let limits = service.limits().clone();
        let concurrency = u32::try_from(limits.max_concurrency).unwrap_or(u32::MAX);
        let builder = tonic::transport::Server::builder()
            .timeout(limits.request_timeout + Duration::from_secs(5))
            .concurrency_limit_per_connection(limits.max_concurrency)
            .max_concurrent_streams(Some(concurrency))
            .load_shed(true);
        let shutdown = async move {
            let _ = grpc_stop.changed().await;
        };
        let mut builder = builder;
        let router = builder
            .add_service(health_service)
            .add_service(geoledger_server::grpc(service));
        match tls {
            Some(tls) => {
                let incoming = tokio_stream::wrappers::ReceiverStream::new(
                    geoledger_server::tls::spawn_acceptor(grpc, tls, true),
                );
                use tokio_stream::StreamExt;
                router
                    .serve_with_incoming_shutdown(
                        incoming.map(|(stream, _)| Ok::<_, std::io::Error>(stream)),
                        shutdown,
                    )
                    .await
            }
            None => {
                router
                    .serve_with_incoming_shutdown(
                        tokio_stream::wrappers::TcpListenerStream::new(grpc),
                        shutdown,
                    )
                    .await
            }
        }
    });
    let deadline = Duration::from_secs(args.shutdown_timeout_secs);
    tokio::select! {
        result = &mut h => {
            let _ = stop.send(true);
            result??;
            bounded(deadline, async { Ok(g.await??) }).await?;
        }
        result = &mut g => {
            let _ = stop.send(true);
            result??;
            bounded(deadline, async { Ok(h.await??) }).await?;
        }
        _ = shutdown() => {
            draining.start_draining();
            health_reporter
                .set_not_serving::<geoledger_rpc::v1::geo_ledger_server::GeoLedgerServer<Service>>()
                .await;
            if args.shutdown_drain_secs > 0 {
                tracing::info!(seconds = args.shutdown_drain_secs, "reporting not ready before shutdown");
                tokio::time::sleep(Duration::from_secs(args.shutdown_drain_secs)).await;
            }
            tracing::info!(timeout_secs = args.shutdown_timeout_secs, "draining requests");
            let _ = stop.send(true);
            bounded(deadline, async {
                h.await??;
                g.await??;
                Ok(())
            })
            .await?;
        }
    }
    tracing::info!("shutdown complete");
    Ok(())
}
/// Wait for listeners to finish, giving up after the shutdown deadline.
async fn bounded(
    deadline: Duration,
    work: impl std::future::Future<Output = Result<(), BoxError>>,
) -> Result<(), BoxError> {
    match tokio::time::timeout(deadline, work).await {
        Ok(result) => result,
        Err(_) => {
            tracing::warn!(
                timeout_secs = deadline.as_secs(),
                "shutdown deadline exceeded; abandoning in-flight requests"
            );
            Err("shutdown deadline exceeded".into())
        }
    }
}
type Fingerprint = Option<(SystemTime, u64)>;
fn fingerprint(path: &Path) -> Fingerprint {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}
/// Reload changed TLS files on an interval and immediately on SIGHUP.
/// A file that fails validation never replaces the active configuration.
async fn reload_loop(tls: Option<Arc<Tls>>, every: Duration) {
    let tls_paths: Vec<PathBuf> = tls
        .as_ref()
        .map(|t| {
            let f = t.files();
            [
                Some(f.cert.clone()),
                Some(f.key.clone()),
                f.client_ca.clone(),
            ]
            .into_iter()
            .flatten()
            .collect()
        })
        .unwrap_or_default();
    let mut tls_seen: Vec<Fingerprint> = tls_paths.iter().map(|p| fingerprint(p)).collect();
    #[cfg(unix)]
    let mut hup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).ok();
    loop {
        #[cfg(unix)]
        let forced = match hup.as_mut() {
            Some(sig) => tokio::select! {_=sig.recv()=>true,_=tokio::time::sleep(every)=>false},
            None => {
                tokio::time::sleep(every).await;
                false
            }
        };
        #[cfg(not(unix))]
        let forced = {
            tokio::time::sleep(every).await;
            false
        };
        if let Some(tls) = &tls {
            let now: Vec<Fingerprint> = tls_paths.iter().map(|p| fingerprint(p)).collect();
            if forced || now != tls_seen {
                tls_seen = now;
                match tls.reload() {
                    Ok(()) => tracing::info!("TLS certificates reloaded"),
                    Err(error) => {
                        tracing::error!(%error, "TLS reload failed; keeping previous certificates")
                    }
                }
            }
        }
    }
}
fn create_private(path: &Path, data: &[u8]) -> Result<(), BoxError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    if let Err(error) = file.write_all(data).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(path);
        return Err(error.into());
    }
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
/// Write a unique, private recovery file before committing the random password.
/// Only the database bootstrap winner retains its file; no existing path is overwritten.
fn initialize_admin(
    app: &Application,
    dir: &Path,
    username: &str,
    password: Option<&str>,
) -> Result<(), BoxError> {
    if !app.bootstrap_needed()? {
        return Ok(());
    }
    if let Some(password) = password {
        app.bootstrap_admin(username, password)?;
        return Ok(());
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "secure random unavailable")?;
    let password: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let path = dir.join(format!("initial-admin-{}.json", uuid::Uuid::new_v4()));
    let data = serde_json::to_vec(&serde_json::json!({"username":username,"password":password}))?;
    create_private(&path, &data)?;
    match app.bootstrap_admin(username, &password) {
        Ok(true) => {
            tracing::warn!(credentials = %path.display(), "initial administrator credentials created; change the password and remove this file")
        }
        Ok(false) => std::fs::remove_file(path)?,
        Err(error) => {
            // COMMIT may have succeeded remotely despite a lost acknowledgement.
            tracing::warn!(credentials = %path.display(), "administrator initialization failed; retained private recovery file");
            return Err(error.into());
        }
    }
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_=sig.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Result<(tempfile::TempDir, Application), BoxError> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/auth-refactor-server-bootstrap");
        std::fs::create_dir_all(&root)?;
        let dir = tempfile::tempdir_in(root)?;
        let app = Application::new(Storage::Sqlite(dir.path().join("db.sqlite3")));
        app.migrate()?;
        Ok((dir, app))
    }
    #[test]
    fn explicit_password_is_idempotent_and_never_written() -> Result<(), BoxError> {
        let (dir, app) = fixture()?;
        initialize_admin(&app, dir.path(), "admin", Some("initial-test-password"))?;
        initialize_admin(&app, dir.path(), "replacement", Some("different-password"))?;
        assert!(
            app.auth_operation(
                None,
                "login_password",
                serde_json::json!({"username":"admin","password":"initial-test-password"})
            )
            .is_ok()
        );
        assert!(std::fs::read_dir(dir.path())?.all(|entry| {
            entry.is_ok_and(|entry| {
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("initial-admin-")
            })
        }));
        Ok(())
    }
    #[test]
    fn failed_bootstrap_retains_private_recovery_file() -> Result<(), BoxError> {
        let (dir, app) = fixture()?;
        assert!(initialize_admin(&app, dir.path(), "invalid username", None).is_err());
        let files: Vec<_> = std::fs::read_dir(dir.path())?
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("initial-admin-")
            })
            .collect();
        assert_eq!(files.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(files[0].metadata()?.permissions().mode() & 0o777, 0o600);
        }
        assert!(app.bootstrap_needed()?);
        Ok(())
    }
    #[test]
    fn concurrent_random_bootstrap_keeps_exactly_one_private_file() -> Result<(), BoxError> {
        let (dir, app) = fixture()?;
        std::thread::scope(|scope| -> Result<(), BoxError> {
            let first = scope.spawn(|| initialize_admin(&app, dir.path(), "admin", None));
            let second = scope.spawn(|| initialize_admin(&app, dir.path(), "admin", None));
            first.join().map_err(|_| "thread panic")??;
            second.join().map_err(|_| "thread panic")??;
            Ok(())
        })?;
        let files: Vec<_> = std::fs::read_dir(dir.path())?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("initial-admin-")
            })
            .collect();
        assert_eq!(files.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(files[0].metadata()?.permissions().mode() & 0o777, 0o600);
        }
        let credentials: serde_json::Value =
            serde_json::from_slice(&std::fs::read(files[0].path())?)?;
        assert_eq!(
            credentials["password"].as_str().ok_or("password")?.len(),
            64
        );
        assert!(
            app.auth_operation(None, "login_password", credentials)
                .is_ok()
        );
        initialize_admin(&app, dir.path(), "admin", None)?;
        Ok(())
    }
}
