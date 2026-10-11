//! Load the operator-installed SpatiaLite extension, then disable extension loading.
//! No database content or remote request controls the library path.
use parking_lot::{RwLock, RwLockReadGuard};
use rusqlite::{Connection as SqliteConnection, LoadExtensionGuard, Result};
use std::{
    ops::{Deref, DerefMut},
    path::Path,
};

// SpatiaLite 5.1 calls libxml2's process-global xmlCleanupParser from its
// per-connection destructor. That routine cannot overlap another native call.
// Queries may run together; initialization and destruction run exclusively.
// Readers must be able to progress while destruction waits: a SQLite writer
// can need another query to release the database lock that another reader is
// waiting on. Writer-preferring read locks would introduce a cross-lock cycle.
// Fresh connections still require exclusive admission. Finite, deadline-bound
// operations drain before a waiting native destructor can run.
static LIFECYCLE: RwLock<()> = RwLock::new(());

/// Owns a loaded connection so native destruction cannot escape lifecycle control.
/// Do not retain an access guard while opening/dropping another loaded connection.
/// The operator-installed extension is trusted; code embedding other libxml2 users
/// must coordinate their lifecycle too. GeoLedger exposes no arbitrary native SQL.
pub struct Connection {
    inner: Option<SqliteConnection>,
}

pub struct Access<'a> {
    connection: &'a SqliteConnection,
    _native: RwLockReadGuard<'static, ()>,
}
impl Deref for Access<'_> {
    type Target = SqliteConnection;
    fn deref(&self) -> &Self::Target {
        self.connection
    }
}
pub struct AccessMut<'a> {
    connection: &'a mut SqliteConnection,
    _native: RwLockReadGuard<'static, ()>,
}
impl Deref for AccessMut<'_> {
    type Target = SqliteConnection;
    fn deref(&self) -> &Self::Target {
        self.connection
    }
}
impl DerefMut for AccessMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.connection
    }
}
impl Connection {
    /// Hold through statement/row/transaction destruction. Independent accesses
    /// share the read lock, so this does not serialize query execution.
    pub fn read(&self) -> Result<Access<'_>> {
        let native = LIFECYCLE.read_recursive();
        Ok(Access {
            connection: self.inner.as_ref().ok_or(rusqlite::Error::InvalidQuery)?,
            _native: native,
        })
    }
    pub fn write(&mut self) -> Result<AccessMut<'_>> {
        let native = LIFECYCLE.read_recursive();
        Ok(AccessMut {
            connection: self.inner.as_mut().ok_or(rusqlite::Error::InvalidQuery)?,
            _native: native,
        })
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _native = LIFECYCLE.write();
        // Take/drop before the writer guard: struct fields would otherwise be
        // destroyed after this Drop method released the lifecycle lock.
        drop(self.inner.take());
    }
}

/// `path` must identify the administrator's trusted SpatiaLite installation.
/// Call before sharing the connection or executing any application SQL.
pub fn load(connection: SqliteConnection, path: &Path) -> Result<Connection> {
    let _native = LIFECYCLE.write();
    match load_extension(&connection, path) {
        Ok(()) => Ok(Connection {
            inner: Some(connection),
        }),
        Err(error) => {
            // A partially loaded extension can already have a destructor.
            drop(connection);
            Err(error)
        }
    }
}
fn load_extension(connection: &SqliteConnection, path: &Path) -> Result<()> {
    // SAFETY: only the operator-configured extension is executed. The guard
    // disables extension loading on success and every error path. Callers own
    // a fresh connection, so no untrusted SQL can execute in this interval.
    unsafe {
        let _guard = LoadExtensionGuard::new(connection)?;
        connection.load_extension(path, Some("sqlite3_modspatialite_init"))?;
    }
    connection.query_row("SELECT spatialite_version()", [], |row| {
        row.get::<_, String>(0)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loading_disabled(connection: &SqliteConnection) {
        let error = connection
            .query_row::<String, _, _>("SELECT load_extension('missing_extension')", [], |row| {
                row.get(0)
            })
            .err();
        assert!(error.is_some_and(|e| e.to_string().contains("not authorized")));
    }

    #[test]
    fn failed_load_disables_extension_loading() -> Result<()> {
        let _native = LIFECYCLE.write();
        let connection = SqliteConnection::open_in_memory()?;
        assert!(
            load_extension(
                &connection,
                Path::new("/nonexistent/geoledger/mod_spatialite")
            )
            .is_err()
        );
        loading_disabled(&connection);
        Ok(())
    }

    #[test]
    fn native_geometry_roundtrip_and_loading_disabled() -> Result<()> {
        let connection = SqliteConnection::open_in_memory()?;
        let path =
            std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
        let connection = load(connection, Path::new(&path))?;
        let access = connection.read()?;
        loading_disabled(&access);
        let (kind, srid, z): (String, i32, f64) = access.query_row(
            "SELECT typeof(g), ST_SRID(g), ST_Z(g) FROM (SELECT SetSRID(GeomFromGeoJSON(?1),4326) AS g)",
            [r#"{"type":"Point","coordinates":[12.34567890123456,23.45678901234567,123.45678901234567]}"#],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(kind, "blob");
        assert_eq!(srid, 4326);
        assert_eq!(z, 123.45678901234567);
        Ok(())
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[test]
    fn concurrent_native_connections_complete_without_global_cleanup_races() -> Result<()> {
        let extension =
            std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
        let start = std::sync::Arc::new(std::sync::Barrier::new(12));
        let mut workers = Vec::new();
        for _ in 0..12 {
            let extension = extension.clone();
            let start = start.clone();
            workers.push(std::thread::spawn(move || -> Result<()> {
                start.wait();
                for _ in 0..40 {
                    let connection =
                        load(SqliteConnection::open_in_memory()?, Path::new(&extension))?;
                    let z: f64 = connection.read()?.query_row(
                        "SELECT ST_Z(GeomFromGeoJSON(?1))",
                        [r#"{"type":"Point","coordinates":[1,2,3]}"#],
                        |row| row.get(0),
                    )?;
                    assert_eq!(z, 3.0);
                }
                Ok(())
            }));
        }
        for worker in workers {
            match worker.join() {
                Ok(result) => result?,
                Err(_) => return Err(rusqlite::Error::InvalidQuery),
            }
        }
        Ok(())
    }
    #[test]
    fn pending_cleanup_does_not_block_existing_query_progress() -> Result<()> {
        use std::{sync::mpsc, time::Duration};
        let extension =
            std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
        let one = load(SqliteConnection::open_in_memory()?, Path::new(&extension))?;
        let two = load(SqliteConnection::open_in_memory()?, Path::new(&extension))?;
        let three = load(SqliteConnection::open_in_memory()?, Path::new(&extension))?;
        let access = one.read()?;
        let (started_tx, started_rx) = mpsc::channel();
        let (dropped_tx, dropped_rx) = mpsc::channel();
        let cleanup = std::thread::spawn(move || {
            let _ = started_tx.send(());
            drop(two);
            let _ = dropped_tx.send(());
        });
        let started = started_rx.recv_timeout(Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(20));
        let (read_tx, read_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let result = three
                .read()
                .and_then(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)));
            let _ = read_tx.send(result);
        });
        let progressed = read_rx.recv_timeout(Duration::from_secs(2));
        let cleanup_blocked = matches!(dropped_rx.try_recv(), Err(mpsc::TryRecvError::Empty));
        // Release before asserting so a regression cannot strand the worker threads.
        drop(access);
        let cleanup_join = cleanup.join();
        let reader_join = reader.join();
        assert!(started.is_ok() && cleanup_join.is_ok() && reader_join.is_ok());
        assert!(cleanup_blocked, "native cleanup overlapped query access");
        assert!(
            matches!(progressed, Ok(Ok(1))),
            "existing reader blocked behind waiting cleanup"
        );
        Ok(())
    }

    #[test]
    fn independent_connections_share_query_access() -> Result<()> {
        let extension =
            std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
        let one = load(SqliteConnection::open_in_memory()?, Path::new(&extension))?;
        let two = load(SqliteConnection::open_in_memory()?, Path::new(&extension))?;
        let first = one.read()?;
        let second = two.read()?;
        assert_eq!(first.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?, 1);
        assert_eq!(second.query_row("SELECT 2", [], |r| r.get::<_, i64>(0))?, 2);
        Ok(())
    }
}
