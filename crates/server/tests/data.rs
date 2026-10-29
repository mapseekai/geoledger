//! SQLite restore failure recovery and protection of existing operator files.
mod common;

use geoledger_engine::{Application, Storage};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn backup(dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let source = dir.join("source.sqlite3");
    common::database(&source)?;
    let input = dir.join("backup.sqlite3");
    Application::new(Storage::Sqlite(source)).backup(&input)?;
    Ok(input)
}

fn restore_command(input: &Path, data_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_geoledger-server"));
    command
        .arg("--data-dir")
        .arg(data_dir)
        .args(["restore", "--input"])
        .arg(input)
        .env_remove("GL_STORAGE")
        .env_remove("GL_DATABASE_URL");
    command
}

fn assert_restored(input: &Path, data_dir: &Path) -> TestResult {
    let output = restore_command(input, data_dir).output()?;
    assert!(
        output.status.success(),
        "restore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!data_dir.join("geoledger.sqlite3.restoring").exists());
    let target = data_dir.join("geoledger.sqlite3");
    assert_eq!(
        Application::new(Storage::Sqlite(target.clone())).data_summary()?,
        Application::new(Storage::Sqlite(input.to_owned())).data_summary()?
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(target)?.permissions().mode() & 0o777, 0o600);
    }
    Ok(())
}

#[test]
fn restore_preserves_existing_database_files_and_staging() -> TestResult {
    let dir = common::tempdir()?;
    let input = backup(dir.path())?;
    for (index, suffix) in ["", "-wal", "-shm", "-journal", ".restoring"]
        .into_iter()
        .enumerate()
    {
        let data_dir = dir.path().join(format!("target-{index}"));
        fs::create_dir(&data_dir)?;
        let existing = data_dir.join(format!("geoledger.sqlite3{suffix}"));
        fs::write(&existing, b"owned by another operation")?;

        let output = restore_command(&input, &data_dir).output()?;

        assert!(!output.status.success(), "restore replaced {suffix}");
        assert_eq!(fs::read(&existing)?, b"owned by another operation");
        if !suffix.is_empty() {
            assert!(!data_dir.join("geoledger.sqlite3").exists());
        }
        if suffix != ".restoring" {
            assert!(!data_dir.join("geoledger.sqlite3.restoring").exists());
        }
        fs::remove_file(existing)?;
        assert_restored(&input, &data_dir)?;
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn restore_copy_failure_removes_staging_and_allows_retry() -> TestResult {
    let dir = common::tempdir()?;
    let input = backup(dir.path())?;
    let original = fs::read(&input)?;
    assert!(original.len() > 4096);
    let data_dir = dir.path().join("target");
    let command = restore_command(&input, &data_dir);
    // Apply the limit only in the child; ignoring SIGXFSZ makes write return EFBIG.
    let output = Command::new("python3")
        .args([
            "-c",
            "import os, resource, signal, sys\nsignal.signal(signal.SIGXFSZ, signal.SIG_IGN)\nresource.setrlimit(resource.RLIMIT_FSIZE, (4096, 4096))\nos.execv(sys.argv[1], sys.argv[1:])",
        ])
        .arg(command.get_program())
        .args(command.get_args())
        .env_remove("GL_STORAGE")
        .env_remove("GL_DATABASE_URL")
        .output()?;

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("File too large"),
        "expected a copy failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!data_dir.join("geoledger.sqlite3").exists());
    assert!(
        !data_dir.join("geoledger.sqlite3.restoring").exists(),
        "a failed copy must not block the next restore"
    );
    assert_restored(&input, &data_dir)?;
    assert_eq!(fs::read(input)?, original);
    Ok(())
}

#[cfg(unix)]
#[test]
fn restore_install_conflict_preserves_target_and_allows_retry() -> TestResult {
    use std::os::unix::fs::symlink;

    let dir = common::tempdir()?;
    let input = backup(dir.path())?;
    let data_dir = dir.path().join("target");
    fs::create_dir(&data_dir)?;
    let target = data_dir.join("geoledger.sqlite3");
    let other = dir.path().join("another-operations-database");
    // A dangling link passes the initial exists() check but prevents hard_link().
    symlink(&other, &target)?;

    let output = restore_command(&input, &data_dir).output()?;

    assert!(!output.status.success());
    assert_eq!(fs::read_link(&target)?, other);
    assert!(!other.exists());
    assert!(
        !data_dir.join("geoledger.sqlite3.restoring").exists(),
        "a failed install must not block the next restore"
    );
    fs::remove_file(target)?;
    assert_restored(&input, &data_dir)
}
