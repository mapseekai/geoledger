#![forbid(unsafe_code)]
use geoledger_center::{CenterApplication, Tokens, router};
fn main() {
    if let Err(message) = run() {
        eprintln!("gl-center: {message}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--help"] || args == ["-h"] {
        println!(
            "gl-center tokens --out FILE SUBJECT... | migrate | serve [--listen 127.0.0.1:7881]\nmigrate/serve require GL_CENTER_DATABASE_URL; serve also requires GL_CENTER_TOKEN_FILE."
        );
        return Ok(());
    }
    if args == ["--build-info"] {
        println!(
            "{}",
            serde_json::json!({
                "product":"geoledger-center", "version":env!("CARGO_PKG_VERSION"),
                "commit":env!("GL_CENTER_BUILD_COMMIT"),
                "source_status":env!("GL_CENTER_BUILD_SOURCE_STATUS"),
                "target":env!("GL_CENTER_BUILD_TARGET"),
                "profile":env!("GL_CENTER_BUILD_PROFILE"),
                "c_runtime":env!("GL_CENTER_BUILD_C_RUNTIME"),
                "rustc":env!("GL_CENTER_BUILD_RUSTC")
            })
        );
        return Ok(());
    }
    if args == ["--version"] || args == ["-V"] {
        println!("gl-center {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "tokens") {
        return create_tokens(&args[1..]);
    }
    let command = args.first().map(String::as_str).unwrap_or_default();
    let listen = match args.as_slice() {
        [cmd] if cmd == "migrate" || cmd == "serve" => "127.0.0.1:7881",
        [cmd, flag, address] if cmd == "serve" && flag == "--listen" => address,
        _ => return Err("usage: gl-center migrate | serve [--listen ADDRESS]".into()),
    };
    let app = CenterApplication::new(
        std::env::var("GL_CENTER_DATABASE_URL")
            .map_err(|_| "GL_CENTER_DATABASE_URL is required")?,
    );
    if command == "migrate" {
        return app.migrate().map_err(|e| e.to_string());
    }
    let file =
        std::env::var("GL_CENTER_TOKEN_FILE").map_err(|_| "GL_CENTER_TOKEN_FILE is required")?;
    let metadata = std::fs::metadata(&file).map_err(|_| "cannot read token file")?;
    if metadata.len() > 1024 * 1024 {
        return Err("token file too large".into());
    }
    let bytes = std::fs::read(file).map_err(|_| "cannot read token file")?;
    let tokens = Tokens::from_json(&bytes).map_err(|e| e.to_string())?;
    let address: std::net::SocketAddr = listen.parse().map_err(|_| "invalid listen address")?;
    app.check_schema().map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Runtime::new().map_err(|_| "cannot start runtime")?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|_| "cannot bind listen address")?;
        let bound = listener
            .local_addr()
            .map_err(|_| "cannot read listener address")?;
        println!("gl-center listening on {bound}");
        axum::serve(listener, router(app, tokens))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .map_err(|_| "HTTP server failed".into())
    })
}

fn create_tokens(args: &[String]) -> Result<(), String> {
    use std::io::Write;
    let [flag, path, subjects @ ..] = args else {
        return Err("usage: gl-center tokens --out FILE SUBJECT...".into());
    };
    if flag != "--out" || subjects.is_empty() || subjects.len() > 1024 {
        return Err("usage: gl-center tokens --out FILE SUBJECT...".into());
    }
    let mut entries = Vec::new();
    for subject in subjects {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| "secure randomness unavailable")?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        entries.push(serde_json::json!({"subject":subject,"token":token}));
    }
    let encoded = serde_json::to_vec_pretty(&entries).map_err(|_| "token encoding failed")?;
    Tokens::from_json(&encoded).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "create a new, writable token file path")?;
    file.write_all(&encoded)
        .map_err(|_| "cannot write token file")?;
    file.sync_all().map_err(|_| "cannot sync token file")?;
    println!(
        "Created token file for {} subjects. Keep it private and distribute each token separately.",
        subjects.len()
    );
    Ok(())
}
