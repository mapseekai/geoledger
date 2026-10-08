use clap::{Parser, Subcommand};
use std::{io::Read, path::PathBuf};
#[derive(Parser)]
#[command(name = "gl", version, about = "Remote GeoLedger client")]
struct Args {
    #[arg(long, env = "GL_ENDPOINT", default_value = "http://127.0.0.1:7882")]
    endpoint: String,
    #[arg(long, env = "GL_TOKEN", hide_env_values = true)]
    token: Option<String>,
    /// Read one credential from a server token file; suitable for local administration.
    #[arg(long, env = "GL_TOKEN_FILE")]
    token_file: Option<PathBuf>,
    #[arg(long, default_value = "admin")]
    subject: String,
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
    let token = match (a.token, a.token_file) {
        (Some(t), None) => t,
        (None, Some(p)) => {
            let mut raw = Vec::new();
            std::fs::File::open(p)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut raw)?;
            if raw.len() > 1024 * 1024 {
                return Err("credential file too large".into());
            }
            let values: Vec<serde_json::Value> = serde_json::from_slice(&raw)?;
            values
                .iter()
                .find(|v| v["subject"] == a.subject)
                .and_then(|v| v["token"].as_str())
                .ok_or("subject not found in token file")?
                .to_owned()
        }
        _ => return Err("supply GL_TOKEN or --token-file, exclusively".into()),
    };
    let (op, input) = match a.command {
        Command::Info => ("info".into(), serde_json::json!({})),
        Command::Projects => ("list_projects".into(), serde_json::json!({"limit":100})),
        Command::History { project, after } => (
            "history".into(),
            serde_json::json!({"project":project,"after":after,"limit":100}),
        ),
        Command::Call { operation, file } => {
            let mut data = Vec::new();
            let reader: Box<dyn Read> = if file.as_os_str() == "-" {
                Box::new(std::io::stdin())
            } else {
                Box::new(std::fs::File::open(file)?)
            };
            reader.take(4 * 1024 * 1024 + 1).read_to_end(&mut data)?;
            if data.len() > 4 * 1024 * 1024 {
                return Err("request too large".into());
            }
            (operation, serde_json::from_slice(&data)?)
        }
    };
    let client = geoledger_client::Client::connect(a.endpoint, &token).await?;
    let result = client.execute(&op, input).await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
