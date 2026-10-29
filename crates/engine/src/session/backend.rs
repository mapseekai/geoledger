use super::*;

pub(crate) struct SqlStorage {
    pub(crate) storage: Storage,
    pub(crate) pool: std::sync::Arc<postgres::Pool>,
}
impl StorageBackend for SqlStorage {
    fn name(&self) -> &'static str {
        match self.storage {
            Storage::Sqlite(_) => "sqlite",
            Storage::Postgis(_) => "postgis",
        }
    }
    fn initialize(&self, timeout: Duration) -> Result<()> {
        Client::open(&self.storage, &self.pool, timeout)?.migrate()
    }
    fn health(&self, timeout: Duration) -> Result<()> {
        let mut c = Client::open(&self.storage, &self.pool, timeout)?;
        if c.query_one("SELECT version FROM gl_format WHERE singleton=true", &[])?
            .get::<_, i32>(0usize)?
            != FORMAT_VERSION
        {
            return Err(Error::new(409, "unsupported storage format"));
        }
        Ok(())
    }
    fn begin(&self, read_only: bool, timeout: Duration) -> Result<Box<dyn RepositoryTransaction>> {
        Ok(Box::new(
            Client::open(&self.storage, &self.pool, timeout)?.transaction(read_only)?,
        ))
    }
    fn pool_stats(&self) -> Option<crate::PoolStats> {
        matches!(self.storage, Storage::Postgis(_)).then(|| self.pool.stats())
    }
}
impl StorageMaintenance for SqlStorage {
    fn export(
        &self,
        out: &mut dyn std::io::Write,
        timeout: Duration,
    ) -> Result<crate::DataSummary> {
        let missing = || Error::new(404, "no GeoLedger database to export");
        if matches!(&self.storage, Storage::Sqlite(path) if !path.is_file()) {
            return Err(missing());
        }
        let mut t = Client::open(&self.storage, &self.pool, timeout)?.transaction(true)?;
        let exists = match self.storage {
            Storage::Sqlite(_) => t
                .query_opt(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_format'",
                    &[],
                )?
                .is_some(),
            Storage::Postgis(_) => t
                .query_one("SELECT to_regclass('public.gl_format') IS NOT NULL", &[])?
                .get::<_, bool>(0usize)?,
        };
        if !exists {
            return Err(missing());
        }
        portable::export(&mut t, self.name(), out)
    }
    fn import(
        &self,
        input: &mut dyn std::io::BufRead,
        timeout: Duration,
    ) -> Result<crate::DataSummary> {
        Client::open(&self.storage, &self.pool, timeout)?.migrate()?;
        portable::import(Client::open(&self.storage, &self.pool, timeout)?, input)
    }
    fn backup(&self, target: &std::path::Path, timeout: Duration) -> Result<()> {
        let Storage::Sqlite(source) = &self.storage else {
            return Err(Error::new(
                409,
                "PostgreSQL backups use pg_dump or WAL archiving (see docs/production.md); `geoledger-server export` gives a portable logical copy",
            ));
        };
        let missing = || Error::new(404, "no GeoLedger database to back up");
        if !source.is_file() {
            return Err(missing());
        }
        let target_text = target
            .to_str()
            .ok_or_else(|| Error::new(400, "backup path must be valid UTF-8"))?
            .to_owned();
        // Reserve the target as a new private file (VACUUM INTO accepts an empty
        // file), so a backup is never readable by others and never overwrites.
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(target).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Error::new(409, "backup target already exists")
            } else {
                Error::new(500, "cannot create backup file").caused_by(e)
            }
        })?;
        let result = (|| {
            let mut c = Client::open(&self.storage, &self.pool, timeout)?;
            let exists = c
                .query_opt(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='gl_format'",
                    &[],
                )?
                .is_some();
            if !exists {
                return Err(missing());
            }
            // VACUUM INTO reads one consistent snapshot while writers continue (WAL).
            c.execute("VACUUM INTO $1", &[&target_text])?;
            drop(c);
            verify_sqlite_file(target, timeout)
        })();
        if result.is_err() {
            // Never leave a file that looks like a usable backup.
            let _ = std::fs::remove_file(target);
        }
        result
    }
}
