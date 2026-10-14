//! Destructive setup is restricted to the explicitly dedicated test database.
use geoledger_engine::{Error, Result};
pub fn reset_database(db: &mut postgres::Client) -> Result<()> {
    let actual: String = db
        .query_one("SELECT current_database()", &[])
        .map_err(|e| Error::new(503, "cannot verify test database").caused_by(e))?
        .get(0);
    if actual != "geoledger_test" {
        return Err(Error::new(
            400,
            "requires dedicated geoledger_test database",
        ));
    }
    db.batch_execute(
        "DROP SCHEMA IF EXISTS business CASCADE; DROP SCHEMA public CASCADE; CREATE SCHEMA public; CREATE EXTENSION IF NOT EXISTS postgis;",
    )
    .map_err(|e| Error::new(503, "test schema reset failed").caused_by(e))?;
    Ok(())
}
pub fn connect_isolated(dsn: &str, _label: &str) -> Result<(String, postgres::Client)> {
    let config: postgres::Config = dsn
        .parse()
        .map_err(|e| Error::new(400, "invalid test database configuration").caused_by(e))?;
    if config.get_dbname() != Some("geoledger_test") {
        return Err(Error::new(
            400,
            "requires dedicated geoledger_test database",
        ));
    }
    let mut db = config
        .connect(postgres::NoTls)
        .map_err(|e| Error::new(503, "test connection failed").caused_by(e))?;
    let actual: String = db
        .query_one("SELECT current_database()", &[])
        .map_err(|e| Error::new(503, "cannot verify test database").caused_by(e))?
        .get(0);
    if actual != "geoledger_test" {
        return Err(Error::new(400, "live database is not geoledger_test"));
    }
    Ok((dsn.to_owned(), db))
}
