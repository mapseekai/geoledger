use clap::{Parser, Subcommand, ValueEnum};
use spatial_version::{Application, Command, Resolution};
use spatial_version_core::{Error, Result};
use spatial_version_postgis::PostgisProvider;
use spatial_version_server::ServerConfig;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

#[derive(Parser)]
#[command(
    name = "spatial-version",
    version,
    about = "PostGIS spatial data version control"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    repo: PathBuf,
    #[arg(long, global = true, env = "SV_AUTHOR", default_value = "unknown")]
    author: String,
    /// Name of an environment variable; credentials are never stored in the repository.
    #[arg(long, global = true, default_value = "SV_DATABASE_URL")]
    database_env: String,
    /// Per-statement PostGIS timeout, including streaming reads during import.
    #[arg(long, global = true, env = "SV_STATEMENT_TIMEOUT_SECS", default_value_t = 120,
        value_parser = clap::value_parser!(u64).range(1..=2_147_483))]
    statement_timeout_secs: u64,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Upgrade legacy datasets to stable column identity tracking (format v2).
    Upgrade,
    Schema {
        dataset: String,
        #[arg(long, default_value = "HEAD")]
        reference: String,
    },
    AddField {
        dataset: String,
        name: String,
        #[arg(long = "type")]
        data_type: String,
        #[arg(short, long)]
        message: Option<String>,
    },
    DropField {
        dataset: String,
        name: String,
        #[arg(long)]
        discard: bool,
        #[arg(short, long)]
        message: Option<String>,
    },
    RenameField {
        dataset: String,
        name: String,
        new_name: String,
        #[arg(short, long)]
        message: Option<String>,
    },
    AlterFieldType {
        dataset: String,
        name: String,
        #[arg(long = "type")]
        data_type: String,
        #[arg(short, long)]
        message: Option<String>,
    },
    Init,
    Import {
        dataset: String,
        #[arg(long, default_value = "public")]
        schema: String,
        #[arg(long)]
        table: String,
        #[arg(short, long)]
        message: Option<String>,
    },
    Status {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Diff {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Commit {
        #[arg(short, long)]
        message: String,
    },
    Log {
        #[arg(default_value = "HEAD")]
        reference: String,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Show {
        #[arg(default_value = "HEAD")]
        reference: String,
        #[arg(long)]
        dataset: Option<String>,
        #[arg(long)]
        key: Option<String>,
    },
    Branch {
        name: Option<String>,
        #[arg(long, default_value = "HEAD")]
        from: String,
    },
    #[command(alias = "checkout")]
    Switch {
        branch: String,
    },
    Restore {
        #[arg(long)]
        discard: bool,
    },
    Reset {
        target: String,
        #[arg(long)]
        hard: bool,
    },
    Merge {
        #[arg(conflicts_with_all=["continue_merge","abort"])]
        source: Option<String>,
        #[arg(long = "continue", conflicts_with = "abort")]
        continue_merge: bool,
        #[arg(long)]
        abort: bool,
        #[arg(short, long)]
        message: Option<String>,
    },
    Revert {
        target: String,
        #[arg(short, long)]
        message: Option<String>,
    },
    Conflicts {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Resolve {
        dataset: String,
        key: String,
        #[arg(long, value_enum)]
        take: Choice,
        #[arg(long)]
        record: Option<PathBuf>,
    },
    Recover,
    Fsck,
    Reflog {
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Start HTTP API and gRPC listeners. Use SV_API_TOKEN for authentication.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7878")]
        http: SocketAddr,
        #[arg(long, default_value = "127.0.0.1:7879")]
        grpc: SocketAddr,
    },
}
#[derive(Clone, Copy, ValueEnum)]
enum Choice {
    Ours,
    Theirs,
    Base,
    Delete,
    Custom,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    match run(Cli::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::json!({"error":{"code":error.code(),"message":error.to_string()}})
            );
            std::process::ExitCode::from(1)
        }
    }
}
async fn run(cli: Cli) -> Result<()> {
    let mut app = Application::new(cli.repo);
    if let Ok(dsn) = std::env::var(&cli.database_env) {
        app = app.with_provider(Arc::new(PostgisProvider::new(dsn).with_statement_timeout(
            std::time::Duration::from_secs(cli.statement_timeout_secs),
        )?));
    }
    let author = cli.author;
    let command = match cli.command {
        Action::Upgrade => Command::Upgrade,
        Action::Schema { dataset, reference } => Command::Schema { dataset, reference },
        Action::AddField {
            dataset,
            name,
            data_type,
            message,
        } => Command::AlterSchema {
            dataset,
            change: spatial_version::core::schema::SchemaEdit::Add { name, data_type },
            author,
            message,
        },
        Action::DropField {
            dataset,
            name,
            discard,
            message,
        } => Command::AlterSchema {
            dataset,
            change: spatial_version::core::schema::SchemaEdit::Drop { name, discard },
            author,
            message,
        },
        Action::RenameField {
            dataset,
            name,
            new_name,
            message,
        } => Command::AlterSchema {
            dataset,
            change: spatial_version::core::schema::SchemaEdit::Rename { name, new_name },
            author,
            message,
        },
        Action::AlterFieldType {
            dataset,
            name,
            data_type,
            message,
        } => Command::AlterSchema {
            dataset,
            change: spatial_version::core::schema::SchemaEdit::AlterType { name, data_type },
            author,
            message,
        },
        Action::Init => Command::Init { author },
        Action::Import {
            dataset,
            schema,
            table,
            message,
        } => Command::Import {
            dataset,
            schema,
            table,
            author,
            message,
        },
        Action::Status { limit } => Command::Status { limit },
        Action::Diff { from, to, limit } => Command::Diff { from, to, limit },
        Action::Commit { message } => Command::Commit { message, author },
        Action::Log { reference, limit } => Command::Log { reference, limit },
        Action::Show {
            reference,
            dataset,
            key,
        } => Command::Show {
            reference,
            dataset,
            key,
        },
        Action::Branch {
            name: Some(name),
            from,
        } => Command::Branch { name, from },
        Action::Branch { name: None, .. } => Command::Branches,
        Action::Switch { branch } => Command::Switch { branch },
        Action::Restore { discard } => Command::Restore { discard },
        Action::Reset { target, hard } => Command::Reset { target, hard },
        Action::Merge {
            continue_merge: true,
            ..
        } => Command::MergeContinue,
        Action::Merge { abort: true, .. } => Command::MergeAbort,
        Action::Merge {
            source: Some(source),
            message,
            ..
        } => Command::Merge {
            source,
            author,
            message,
        },
        Action::Merge { .. } => {
            return Err(Error::Invalid(
                "provide a source branch, --continue or --abort".into(),
            ));
        }
        Action::Revert { target, message } => Command::Revert {
            target,
            author,
            message,
        },
        Action::Conflicts { limit } => Command::Conflicts { limit },
        Action::Resolve {
            dataset,
            key,
            take,
            record,
        } => {
            let choice = match take {
                Choice::Ours => Resolution::Ours,
                Choice::Theirs => Resolution::Theirs,
                Choice::Base => Resolution::Base,
                Choice::Delete => Resolution::Delete,
                Choice::Custom => Resolution::Custom,
            };
            let record = record
                .map(|p| {
                    if std::fs::metadata(&p)?.len()
                        > spatial_version_server::MAX_REQUEST_BYTES as u64
                    {
                        return Err(Error::Invalid("resolution file exceeds 4 MiB".into()));
                    }
                    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
                })
                .transpose()?;
            Command::Resolve {
                dataset,
                key,
                choice,
                record,
            }
        }
        Action::Recover => Command::Recover,
        Action::Fsck => Command::Fsck,
        Action::Reflog { limit } => Command::Reflog { limit },
        Action::Serve { http, grpc } => {
            return spatial_version_server::serve(
                app,
                ServerConfig {
                    http,
                    grpc,
                    token: std::env::var("SV_API_TOKEN").ok(),
                },
            )
            .await;
        }
    };
    let result = tokio::task::spawn_blocking(move || app.execute(command))
        .await
        .map_err(|e| Error::storage_source("operation worker failed", e))??;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
