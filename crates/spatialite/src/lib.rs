//! Load the operator-installed SpatiaLite extension, then disable extension loading.
//! No database content or remote request controls the library path.
use rusqlite::{Connection, LoadExtensionGuard, Result};
use std::path::Path;

/// `path` must identify the administrator's trusted SpatiaLite installation.
/// Call before sharing the connection or executing any application SQL.
pub fn load(connection: &Connection, path: &Path) -> Result<()> {
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

    fn loading_disabled(connection: &Connection) {
        let error = connection
            .query_row::<String, _, _>("SELECT load_extension('missing_extension')", [], |row| {
                row.get(0)
            })
            .err();
        assert!(error.is_some_and(|e| e.to_string().contains("not authorized")));
    }

    #[test]
    fn failed_load_disables_extension_loading() -> Result<()> {
        let connection = Connection::open_in_memory()?;
        assert!(
            load(
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
        let connection = Connection::open_in_memory()?;
        let path =
            std::env::var_os("GL_SPATIALITE_EXTENSION").unwrap_or_else(|| "mod_spatialite".into());
        load(&connection, Path::new(&path))?;
        loading_disabled(&connection);
        let (kind, srid, z): (String, i32, f64) = connection.query_row(
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
