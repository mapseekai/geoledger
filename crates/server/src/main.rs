use clap::{Parser, Subcommand, ValueEnum};
use geoledger_server::{Application, Authentication, JwtAuthenticator, Service, Storage, Tokens};
use std::{
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};
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
    #[arg(long, env = "GL_HTTP_LISTEN", default_value = "127.0.0.1:7881")]
    http: SocketAddr,
    #[arg(long, env = "GL_GRPC_LISTEN", default_value = "127.0.0.1:7882")]
    grpc: SocketAddr,
    #[arg(long, env = "GL_TOKEN_FILE")]
    token_file: Option<PathBuf>,
    #[arg(long, env = "GL_JWKS_FILE")]
    jwks_file: Option<PathBuf>,
    #[arg(long, env = "GL_JWT_ISSUER")]
    jwt_issuer: Option<String>,
    #[arg(long, env = "GL_JWT_AUDIENCE")]
    jwt_audience: Option<String>,
}
#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    Sqlite,
    Postgis,
}
#[derive(Subcommand)]
enum Command {
    /// Generate a new secret file; does not overwrite existing files.
    Tokens {
        #[arg(long)]
        out: PathBuf,
        #[arg(required = true)]
        subjects: Vec<String>,
    },
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "geoledger_server=info".into()),
        )
        .init();
    if let Err(e) = run(Args::parse()).await {
        eprintln!("geoledger-server: {e}");
        std::process::exit(1);
    }
}
async fn run(args: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(Command::Tokens { out, subjects }) = args.command {
        return tokens(&out, &subjects);
    }
    if args.jwks_file.is_some() && args.token_file.is_some() {
        return Err("choose JWT or static tokens".into());
    }
    if args.jwks_file.is_none() && (args.jwt_issuer.is_some() || args.jwt_audience.is_some()) {
        return Err("JWT issuer/audience require a JWKS file".into());
    }
    if matches!(args.storage, Backend::Sqlite) && args.database_url.is_some() {
        return Err(
            "GL_DATABASE_URL requires --storage postgis; refusing to silently use SQLite".into(),
        );
    }
    std::fs::create_dir_all(&args.data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&args.data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let storage = match args.storage {
        Backend::Sqlite => Storage::Sqlite(args.data_dir.join("geoledger.sqlite3")),
        Backend::Postgis => Storage::Postgis(
            args.database_url
                .ok_or("PostGIS requires GL_DATABASE_URL")?,
        ),
    };
    let authentication = if let Some(path) = args.jwks_file {
        Authentication::Jwt(Box::new(JwtAuthenticator::from_jwks(
            &read(&path)?,
            &args.jwt_issuer.ok_or("JWT issuer required")?,
            &args.jwt_audience.ok_or("JWT audience required")?,
        )?))
    } else {
        let explicit_token_file = args.token_file.is_some();
        let path = args
            .token_file
            .unwrap_or_else(|| args.data_dir.join("tokens.json"));
        if !path.exists() {
            if explicit_token_file {
                return Err("configured token file does not exist".into());
            }
            tokens(&path, &["admin".into()])?;
            tracing::info!(path=%path.display(),"created initial admin credentials; read token file locally");
        }
        Authentication::Tokens(Tokens::from_json(&read(&path)?)?)
    };
    let app = Application::new(storage);
    let bootstrap = app.clone();
    tokio::task::spawn_blocking(move || bootstrap.migrate()).await??;
    let backend = app.backend();
    let service = Service::new(app, authentication);
    // Bind both before reporting readiness. A failed listener leaves no half-started service.
    let http = tokio::net::TcpListener::bind(args.http).await?;
    let grpc = tokio::net::TcpListener::bind(args.grpc).await?;
    tracing::info!(http=%http.local_addr()?,grpc=%grpc.local_addr()?,%backend,"GeoLedger ready");
    let (stop, rx) = tokio::sync::watch::channel(false);
    let mut http_stop = rx.clone();
    let mut grpc_stop = rx;
    let router = geoledger_server::router(service.clone());
    let mut h = tokio::spawn(async move {
        axum::serve(http, router)
            .with_graceful_shutdown(async move {
                let _ = http_stop.changed().await;
            })
            .await
    });
    let mut g = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .timeout(std::time::Duration::from_secs(30))
            .concurrency_limit_per_connection(20)
            .max_concurrent_streams(Some(20))
            .load_shed(true)
            .add_service(geoledger_server::grpc(service))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(grpc),
                async move {
                    let _ = grpc_stop.changed().await;
                },
            )
            .await
    });
    tokio::select! {
     result=&mut h=>{let _=stop.send(true);result??;g.await??;},
     result=&mut g=>{let _=stop.send(true);result??;h.await??;},
     _=shutdown()=>{tracing::info!("draining requests");let _=stop.send(true);h.await??;g.await??;}
    }
    Ok(())
}
fn read(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut v = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut v)?;
    if v.len() > 1024 * 1024 {
        return Err("authentication file too large".into());
    }
    Ok(v)
}
fn tokens(
    path: &Path,
    subjects: &[String],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut entries = Vec::new();
    for subject in subjects {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| "secure random unavailable")?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        entries.push(serde_json::json!({"subject":subject,"token":token}));
    }
    let data = serde_json::to_vec_pretty(&entries)?;
    Tokens::from_json(&data)?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(&data)?;
    file.sync_all()?;
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
