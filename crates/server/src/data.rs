//! Operator commands: export, import, verify, backup and restore.
use super::BoxError;
use geoledger_engine::{Application, DataSummary};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::Duration,
};

fn stdio(path: &Path) -> bool {
    path.as_os_str() == "-"
}
/// New private file; never truncates an existing one.
fn create_private(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
fn print(summary: &DataSummary, to_stderr: bool) -> Result<(), BoxError> {
    let text = serde_json::to_string_pretty(summary)?;
    if to_stderr {
        eprintln!("{text}");
    } else {
        println!("{text}");
    }
    Ok(())
}
fn reader(path: &Path) -> Result<Box<dyn BufRead + Send>, BoxError> {
    Ok(if stdio(path) {
        Box::new(BufReader::new(std::io::stdin()))
    } else {
        Box::new(BufReader::with_capacity(1 << 20, File::open(path)?))
    })
}

pub(crate) async fn export(app: Application, output: PathBuf) -> Result<(), BoxError> {
    let to_stdout = stdio(&output);
    let summary = tokio::task::spawn_blocking(move || -> Result<DataSummary, BoxError> {
        if to_stdout {
            let mut out = BufWriter::new(std::io::stdout().lock());
            return Ok(app.export_data(&mut out)?);
        }
        let file = create_private(&output)
            .map_err(|e| format!("cannot create {}: {e}", output.display()))?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        let result = app
            .export_data(&mut out)
            .map_err(BoxError::from)
            .and_then(|s| {
                let file = out.into_inner().map_err(|e| e.into_error())?;
                file.sync_all()?;
                Ok(s)
            });
        if result.is_err() {
            // Never leave a partial export that looks complete.
            let _ = std::fs::remove_file(&output);
        }
        result
    })
    .await??;
    print(&summary, to_stdout)
}

pub(crate) async fn import(app: Application, input: PathBuf) -> Result<(), BoxError> {
    let summary = tokio::task::spawn_blocking(move || -> Result<DataSummary, BoxError> {
        Ok(app.import_data(&mut reader(&input)?)?)
    })
    .await??;
    print(&summary, false)
}

pub(crate) async fn verify(
    app: Application,
    input: PathBuf,
    file_only: bool,
) -> Result<(), BoxError> {
    let (file, database) = tokio::task::spawn_blocking(
        move || -> Result<(DataSummary, Option<DataSummary>), BoxError> {
            let file = Application::read_export(&mut reader(&input)?)?;
            let database = if file_only {
                None
            } else {
                Some(app.data_summary()?)
            };
            Ok((file, database))
        },
    )
    .await??;
    match database {
        None => print(&file, false),
        Some(database) if database == file => {
            println!("export matches the database");
            print(&file, false)
        }
        Some(database) => {
            for (f, d) in file.tables.iter().zip(&database.tables) {
                if f != d {
                    eprintln!(
                        "{}: export {} rows / database {} rows (digest {})",
                        f.table,
                        f.rows,
                        d.rows,
                        if f.digest == d.digest {
                            "equal"
                        } else {
                            "differs"
                        }
                    );
                }
            }
            eprintln!("export does not match the database");
            std::process::exit(4);
        }
    }
}

pub(crate) async fn backup(app: Application, output: PathBuf) -> Result<(), BoxError> {
    let summary = tokio::task::spawn_blocking(move || -> Result<DataSummary, BoxError> {
        app.backup(&output)?;
        let copy = Application::new(geoledger_engine::Storage::Sqlite(output.clone()));
        Ok(copy.data_summary()?)
    })
    .await??;
    print(&summary, false)
}

pub(crate) async fn restore(
    input: PathBuf,
    target: PathBuf,
    timeout: Duration,
) -> Result<(), BoxError> {
    let summary = tokio::task::spawn_blocking(move || -> Result<DataSummary, BoxError> {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = target.clone().into_os_string();
            path.push(suffix);
            if Path::new(&path).exists() {
                return Err(format!(
                    "{} exists; restore only into an empty data directory (move the old database aside first)",
                    Path::new(&path).display()
                )
                .into());
            }
        }
        Application::verify_sqlite_file(&input, timeout)?;
        let mut staging = target.clone().into_os_string();
        staging.push(".restoring");
        let staging = PathBuf::from(staging);
        {
            let mut source = File::open(&input)?;
            let mut copy = create_private(&staging)
                .map_err(|e| format!("cannot create {}: {e}", staging.display()))?;
            std::io::copy(&mut source, &mut copy)?;
            copy.flush()?;
            copy.sync_all()?;
        }
        let checked = Application::verify_sqlite_file(&staging, timeout)
            .map_err(BoxError::from)
            .and_then(|()| {
                Ok(Application::new(geoledger_engine::Storage::Sqlite(staging.clone()))
                    .with_timeout(timeout)
                    .data_summary()?)
            });
        let summary = match checked {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_file(&staging);
                return Err(e);
            }
        };
        std::fs::rename(&staging, &target)?;
        if let Some(dir) = target.parent() {
            #[cfg(unix)]
            File::open(dir)?.sync_all()?;
            let _ = dir;
        }
        Ok(summary)
    })
    .await??;
    print(&summary, false)
}
