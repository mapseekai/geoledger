use clap::{Parser, Subcommand, ValueEnum};
use geoledger_server::{
    Application, Authentication, Authenticator, JwtAuthenticator, Service, Storage, Tokens,
    tls::{Tls, TlsFiles, TlsListener},
};
use std::{
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
type BoxError = Box<dyn std::error::Error + Send + Sync>;
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
    #[arg(long, env = "GL_TOKEN_FILE")]
    token_file: Option<PathBuf>,
    /// Create tokens.json and admin-credentials.json in the data directory when no token file exists.
    #[arg(long, env = "GL_BOOTSTRAP_ADMIN", default_value_t = true, action = clap::ArgAction::Set)]
    bootstrap_admin: bool,
    #[arg(long, env = "GL_JWKS_FILE")]
    jwks_file: Option<PathBuf>,
    /// HTTPS JWKS endpoint, refreshed every --jwks-refresh-secs; the last good key set is kept on failure.
    #[arg(long, env = "GL_JWKS_URL")]
    jwks_url: Option<String>,
    #[arg(long, env = "GL_JWKS_REFRESH_SECS", default_value_t = 300)]
    jwks_refresh_secs: u64,
    #[arg(long, env = "GL_JWT_ISSUER")]
    jwt_issuer: Option<String>,
    #[arg(long, env = "GL_JWT_AUDIENCE")]
    jwt_audience: Option<String>,
    /// How often token, JWKS and TLS files are checked for changes (SIGHUP reloads immediately).
    #[arg(long, env = "GL_RELOAD_INTERVAL_SECS", default_value_t = 10)]
    reload_interval_secs: u64,
}
#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Sqlite,
    Postgis,
}
#[derive(Subcommand)]
enum Command {
    /// Generate credentials. The server file stores SHA-256 digests only; plaintext tokens
    /// go to --client-out (mode 0600) or stdout. Existing files are never overwritten.
    Tokens {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        client_out: Option<PathBuf>,
        /// RFC 3339 expiry applied to every generated token.
        #[arg(long)]
        expires_at: Option<String>,
        #[arg(required = true)]
        subjects: Vec<String>,
    },
    /// Convert a legacy plaintext token file into the hashed format.
    HashTokens {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "geoledger_server=info,geoledger_engine=info".into()),
        )
        .init();
    if let Err(e) = run(Args::parse()).await {
        eprintln!("geoledger-server: {e}");
        std::process::exit(1);
    }
}
/// Where authentication material comes from, for reloads.
#[derive(Clone)]
enum AuthSource {
    TokenFile(PathBuf),
    JwksFile {
        path: PathBuf,
        issuer: String,
        audience: String,
    },
    JwksUrl {
        url: String,
        issuer: String,
        audience: String,
    },
}
fn load_tokens(path: &Path) -> Result<Authenticator, BoxError> {
    let tokens = Tokens::from_json(&read(path)?)?;
    if tokens.legacy_entries() > 0 {
        tracing::warn!(
            path = %path.display(),
            entries = tokens.legacy_entries(),
            "token file contains plaintext tokens; convert with `geoledger-server hash-tokens`"
        );
    }
    Ok(Authenticator::Tokens(tokens))
}
fn load_jwks(bytes: &[u8], issuer: &str, audience: &str) -> Result<Authenticator, BoxError> {
    Ok(Authenticator::Jwt(Box::new(JwtAuthenticator::from_jwks(
        bytes, issuer, audience,
    )?)))
}
async fn run(args: Args) -> Result<(), BoxError> {
    match args.command {
        Some(Command::Tokens {
            out,
            client_out,
            expires_at,
            subjects,
        }) => {
            let issued = tokens(
                &out,
                client_out.as_deref(),
                expires_at.as_deref(),
                &subjects,
            )?;
            if client_out.is_none() {
                for (subject, token) in issued {
                    println!("{subject}\t{token}");
                }
            }
            return Ok(());
        }
        Some(Command::HashTokens { input, out }) => return hash_tokens(&input, &out),
        None => {}
    }
    let jwt = args.jwks_file.is_some() || args.jwks_url.is_some();
    if args.jwks_file.is_some() && args.jwks_url.is_some() {
        return Err("choose GL_JWKS_FILE or GL_JWKS_URL".into());
    }
    if jwt && args.token_file.is_some() {
        return Err("choose JWT or static tokens".into());
    }
    if !jwt && (args.jwt_issuer.is_some() || args.jwt_audience.is_some()) {
        return Err("JWT issuer/audience require a JWKS file or URL".into());
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
    let tls = match (&args.tls_cert, &args.tls_key) {
        (Some(cert), Some(key)) => Some(Tls::load(TlsFiles {
            cert: cert.clone(),
            key: key.clone(),
            client_ca: args.tls_client_ca.clone(),
        })?),
        _ => None,
    };
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let (source, initial) = if let Some(path) = args.jwks_file.clone() {
        let issuer = args.jwt_issuer.clone().ok_or("JWT issuer required")?;
        let audience = args.jwt_audience.clone().ok_or("JWT audience required")?;
        let initial = load_jwks(&read(&path)?, &issuer, &audience)?;
        (
            AuthSource::JwksFile {
                path,
                issuer,
                audience,
            },
            initial,
        )
    } else if let Some(url) = args.jwks_url.clone() {
        let issuer = args.jwt_issuer.clone().ok_or("JWT issuer required")?;
        let audience = args.jwt_audience.clone().ok_or("JWT audience required")?;
        let initial = load_jwks(
            &geoledger_server::fetch_jwks(&http_client, &url).await?,
            &issuer,
            &audience,
        )?;
        (
            AuthSource::JwksUrl {
                url,
                issuer,
                audience,
            },
            initial,
        )
    } else {
        let explicit_token_file = args.token_file.is_some();
        let path = args
            .token_file
            .clone()
            .unwrap_or_else(|| args.data_dir.join("tokens.json"));
        if !path.exists() {
            if explicit_token_file {
                return Err("configured token file does not exist".into());
            }
            if !args.bootstrap_admin {
                return Err("no token file; create one with `geoledger-server tokens` or enable GL_BOOTSTRAP_ADMIN".into());
            }
            let client = args.data_dir.join("admin-credentials.json");
            tokens(&path, Some(&client), None, &["admin".into()])?;
            tracing::warn!(
                credentials = %client.display(),
                "created initial admin credential; copy it to the operator and delete the file"
            );
        }
        let initial = load_tokens(&path)?;
        (AuthSource::TokenFile(path), initial)
    };
    let authentication = Arc::new(Authentication::new(initial));
    let app = Application::new(storage);
    let bootstrap = app.clone();
    tokio::task::spawn_blocking(move || bootstrap.migrate()).await??;
    let backend = app.backend();
    let service = Service::with_authentication(app, authentication.clone());
    // Bind both before reporting readiness. A failed listener leaves no half-started service.
    let http = tokio::net::TcpListener::bind(args.http).await?;
    let grpc = tokio::net::TcpListener::bind(args.grpc).await?;
    let http_addr = http.local_addr()?;
    let grpc_addr = grpc.local_addr()?;
    if tls.is_none() && !(http_addr.ip().is_loopback() && grpc_addr.ip().is_loopback()) {
        tracing::warn!(
            "listening on a non-loopback address without TLS; terminate TLS at a trusted gateway or set GL_TLS_CERT/GL_TLS_KEY"
        );
    }
    tokio::spawn(reload_loop(
        source.clone(),
        authentication.clone(),
        tls.clone(),
        Duration::from_secs(args.reload_interval_secs.max(1)),
    ));
    if let AuthSource::JwksUrl {
        url,
        issuer,
        audience,
    } = source
    {
        let auth = authentication.clone();
        let every = Duration::from_secs(args.jwks_refresh_secs.max(1));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                match geoledger_server::fetch_jwks(&http_client, &url)
                    .await
                    .map_err(BoxError::from)
                    .and_then(|bytes| load_jwks(&bytes, &issuer, &audience))
                {
                    Ok(next) => {
                        auth.replace(next);
                        tracing::debug!("JWKS refreshed");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "JWKS refresh failed; keeping previous keys")
                    }
                }
            }
        });
    }
    tracing::info!(http=%http_addr,grpc=%grpc_addr,%backend,tls=tls.is_some(),mtls=args.tls_client_ca.is_some(),"GeoLedger ready");
    let (stop, rx) = tokio::sync::watch::channel(false);
    let mut http_stop = rx.clone();
    let mut grpc_stop = rx;
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
        let builder = tonic::transport::Server::builder()
            .timeout(Duration::from_secs(30))
            .concurrency_limit_per_connection(20)
            .max_concurrent_streams(Some(20))
            .load_shed(true);
        let shutdown = async move {
            let _ = grpc_stop.changed().await;
        };
        let mut builder = builder;
        let router = builder.add_service(geoledger_server::grpc(service));
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
    tokio::select! {
     result=&mut h=>{let _=stop.send(true);result??;g.await??;},
     result=&mut g=>{let _=stop.send(true);result??;h.await??;},
     _=shutdown()=>{tracing::info!("draining requests");let _=stop.send(true);h.await??;g.await??;}
    }
    Ok(())
}
type Fingerprint = Option<(SystemTime, u64)>;
fn fingerprint(path: &Path) -> Fingerprint {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}
/// Reload changed token/JWKS/TLS files on an interval and immediately on SIGHUP.
/// A file that fails validation never replaces the active configuration.
async fn reload_loop(
    source: AuthSource,
    auth: Arc<Authentication>,
    tls: Option<Arc<Tls>>,
    every: Duration,
) {
    let auth_path = match &source {
        AuthSource::TokenFile(p) => Some(p.clone()),
        AuthSource::JwksFile { path, .. } => Some(path.clone()),
        AuthSource::JwksUrl { .. } => None,
    };
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
    let mut auth_seen = auth_path.as_deref().map(fingerprint);
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
        if let Some(path) = &auth_path {
            let now = fingerprint(path);
            if forced || Some(now) != auth_seen {
                auth_seen = Some(now);
                let next = match &source {
                    AuthSource::TokenFile(p) => load_tokens(p),
                    AuthSource::JwksFile {
                        path,
                        issuer,
                        audience,
                    } => read(path).and_then(|b| load_jwks(&b, issuer, audience)),
                    AuthSource::JwksUrl { .. } => Err("unreachable".into()),
                };
                match next {
                    Ok(next) => {
                        auth.replace(next);
                        tracing::info!(path=%path.display(), "authentication reloaded");
                    }
                    Err(error) => {
                        tracing::error!(path=%path.display(), %error, "authentication reload failed; keeping previous configuration")
                    }
                }
            }
        }
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
fn read(path: &Path) -> Result<Vec<u8>, BoxError> {
    let mut v = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut v)?;
    if v.len() > 1024 * 1024 {
        return Err("authentication file too large".into());
    }
    Ok(v)
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
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}
fn tokens(
    path: &Path,
    client: Option<&Path>,
    expires_at: Option<&str>,
    subjects: &[String],
) -> Result<Vec<(String, String)>, BoxError> {
    if path.exists() || client.is_some_and(Path::exists) {
        return Err("refusing to overwrite an existing credential file".into());
    }
    let mut server = Vec::new();
    let mut issued = Vec::new();
    for subject in subjects {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| "secure random unavailable")?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut entry = serde_json::json!({
            "subject": subject,
            "token_sha256": geoledger_server::sha256_hex(&token),
        });
        if let Some(t) = expires_at {
            entry["expires_at"] = t.into();
        }
        server.push(entry);
        issued.push((subject.clone(), token));
    }
    let data = serde_json::to_vec_pretty(&server)?;
    Tokens::from_json(&data)?;
    if let Some(client) = client {
        let entries: Vec<_> = issued
            .iter()
            .map(|(subject, token)| serde_json::json!({"subject":subject,"token":token}))
            .collect();
        create_private(client, &serde_json::to_vec_pretty(&entries)?)?;
    }
    create_private(path, &data)?;
    Ok(issued)
}
fn hash_tokens(input: &Path, out: &Path) -> Result<(), BoxError> {
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&read(input)?)?;
    let mut converted = Vec::new();
    for mut entry in entries {
        let object = entry
            .as_object_mut()
            .ok_or("token entries must be objects")?;
        if let Some(token) = object.remove("token") {
            let token = token.as_str().ok_or("token must be a string")?;
            object.insert(
                "token_sha256".into(),
                geoledger_server::sha256_hex(token).into(),
            );
        }
        converted.push(entry);
    }
    let data = serde_json::to_vec_pretty(&converted)?;
    Tokens::from_json(&data)?;
    create_private(out, &data)
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
