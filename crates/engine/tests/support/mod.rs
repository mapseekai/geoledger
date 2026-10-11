//! Shared test-database targeting, independent of the input DSN grammar.
use geoledger_engine::{Error, Result};
pub fn connect_isolated(dsn: &str, name: &str) -> Result<(String, postgres::Client)> {
    if !name.starts_with("geoledger_")
        || name == "geoledger_test"
        || name.len() > 63
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(Error::new(400, "invalid isolated test database name"));
    }
    let mut config: postgres::Config = dsn
        .parse()
        .map_err(|e| Error::new(400, "invalid test database configuration").caused_by(e))?;
    config.dbname(name);
    let target = if dsn.starts_with("postgres://") || dsn.starts_with("postgresql://") {
        format!(
            "{}{}dbname={name}",
            dsn,
            if dsn.contains('?') { "&" } else { "?" }
        )
    } else {
        format!("{dsn} dbname={name}")
    };
    let verified: postgres::Config = target
        .parse()
        .map_err(|e| Error::new(400, "invalid isolated target").caused_by(e))?;
    if verified.get_dbname() != Some(name) {
        return Err(Error::new(400, "engine target is not isolated"));
    }
    let mut db = config
        .connect(postgres::NoTls)
        .map_err(|e| Error::new(503, "isolated test connection failed").caused_by(e))?;
    let row = db
        .query_one("SELECT current_database()", &[])
        .map_err(|e| Error::new(503, "cannot verify isolated test target").caused_by(e))?;
    let actual: String = row.get(0);
    if actual != name {
        return Err(Error::new(400, "live test database is not isolated"));
    }
    Ok((target, db))
}
