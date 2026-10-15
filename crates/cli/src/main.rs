use clap::{Parser, Subcommand};
use std::{io::Read, path::PathBuf};
#[derive(Parser)]
#[command(name = "gl", version, about = "Remote GeoLedger client")]
struct Args {
    #[arg(long, env = "GL_ENDPOINT", default_value = "http://127.0.0.1:7882")]
    endpoint: String,
    /// Read an API Key from a private plain-text file (mode 0600).
    #[arg(long, env = "GL_API_KEY_FILE")]
    api_key_file: Option<PathBuf>,
    /// Allow plaintext http:// to a non-loopback host (trusted networks only).
    #[arg(long, env = "GL_ALLOW_INSECURE_TRANSPORT", default_value_t = false)]
    allow_insecure: bool,
    /// Extra PEM CA bundle for https:// endpoints signed by a private CA.
    #[arg(long, env = "GL_CA_FILE")]
    ca_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Show protocol version and storage backend.
    Info,
    /// Invoke a business operation using ordinary JSON fields. '-' reads stdin.
    Call {
        operation: String,
        #[arg(long, default_value = "-")]
        file: PathBuf,
    },
    /// List accessible projects.
    Projects,
    /// List a project's active members.
    Members { project: String },
    /// List a project's published revisions.
    History {
        project: String,
        #[arg(long, default_value_t = 0)]
        after: i64,
    },
}
#[tokio::main]
async fn main() {
    if let Err(e) = run(Args::parse()).await {
        if let Some(detail) = e.downcast_ref::<geoledger_client::Error>() {
            eprintln!(
                "{}",
                serde_json::to_string(detail).unwrap_or_else(|_| "client error".into())
            );
        } else {
            eprintln!("gl: {e}");
        }
        std::process::exit(1);
    }
}
async fn run(a: Args) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api_key = read_api_key(std::env::var("GL_API_KEY").ok(), a.api_key_file)?;
    let (op, input) = match a.command {
        Command::Info => ("info".into(), serde_json::json!({})),
        Command::Projects => ("list_projects".into(), serde_json::json!({"limit":100})),
        Command::Members { project } => (
            "list_members".into(),
            serde_json::json!({"project":project,"limit":1000}),
        ),
        Command::History { project, after } => (
            "history".into(),
            serde_json::json!({"project":project,"after":after,"limit":100}),
        ),
        Command::Call { operation, file } => {
            let mut data = Vec::new();
            let mut reader: Box<dyn Read> = if file.as_os_str() == "-" {
                Box::new(std::io::stdin())
            } else {
                Box::new(std::fs::File::open(file)?)
            };
            reader.read_to_end(&mut data)?;
            (operation, serde_json::from_slice(&data)?)
        }
    };
    let options = geoledger_client::ConnectOptions {
        allow_insecure: a.allow_insecure,
        ca_pem: a.ca_file.map(std::fs::read).transpose()?,
        ..Default::default()
    };
    let client = geoledger_client::Client::connect_with(a.endpoint, &api_key, options).await?;
    let result = client.execute(&op, input).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn read_api_key(
    environment: Option<String>,
    path: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match (environment, path) {
        (Some(key), None) => Ok(key),
        (None, Some(path)) => {
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("API Key file must be a regular file".into());
            }
            let file = std::fs::File::open(&path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let opened = file.metadata()?;
                if opened.mode() & 0o077 != 0
                    || metadata.dev() != opened.dev()
                    || metadata.ino() != opened.ino()
                {
                    return Err(
                        "API Key file must be private (mode 0600) and unchanged while opening"
                            .into(),
                    );
                }
            }
            let mut raw = String::new();
            file.take(1025).read_to_string(&mut raw)?;
            if raw.len() > 1024 {
                return Err("API Key file too large".into());
            }
            Ok(raw.trim().to_owned())
        }
        _ => Err("supply GL_API_KEY or --api-key-file exclusively".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_require_one_source() {
        assert!(read_api_key(None, None).is_err());
        assert!(read_api_key(Some("secret".into()), Some("unused".into())).is_err());
        assert_eq!(
            read_api_key(Some("value".into()), None).ok().as_deref(),
            Some("value")
        );
    }
    #[cfg(unix)]
    #[test]
    fn credential_files_are_private_regular_and_bounded() -> Result<(), Box<dyn std::error::Error>>
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/auth-refactor-cli");
        std::fs::create_dir_all(&root)?;
        let path = root.join(format!("key-{}", std::process::id()));
        std::fs::write(&path, "test-only\n")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        assert_eq!(
            read_api_key(None, Some(path.clone())).ok().as_deref(),
            Some("test-only")
        );
        let link = path.with_extension("link");
        symlink(&path, &link)?;
        assert!(read_api_key(None, Some(link.clone())).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        assert!(read_api_key(None, Some(path.clone())).is_err());
        std::fs::remove_file(link)?;
        std::fs::remove_file(path)?;
        Ok(())
    }
}
