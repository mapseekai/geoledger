use super::RepositoryTransaction;
use crate::Result;
use std::time::Duration;

/// Backend lifecycle and transaction admission; data maintenance is a separate capability.
pub trait StorageBackend: StorageMaintenance + Send + Sync {
    fn name(&self) -> &'static str;

    fn initialize(&self, timeout: Duration) -> Result<()>;

    fn health(&self, timeout: Duration) -> Result<()>;

    fn begin(&self, read_only: bool, timeout: Duration) -> Result<Box<dyn RepositoryTransaction>>;

    /// Connection pool occupancy for metrics; backends without a pool return None.
    fn pool_stats(&self) -> Option<crate::PoolStats> {
        None
    }
}

/// Optional consistent export/import and whole-database backup operations.
pub trait StorageMaintenance: Send + Sync {
    /// Export project/version tables from one consistent snapshot; global credentials are excluded.
    fn export(
        &self,
        out: &mut dyn std::io::Write,
        timeout: Duration,
    ) -> Result<crate::DataSummary> {
        let _ = (out, timeout);
        Err(crate::Error::new(409, "this storage backend has no export"))
    }

    /// Load a validated export into an initialized, empty database atomically.
    fn import(
        &self,
        input: &mut dyn std::io::BufRead,
        timeout: Duration,
    ) -> Result<crate::DataSummary> {
        let _ = (input, timeout);
        Err(crate::Error::new(409, "this storage backend has no import"))
    }

    /// Online, consistent copy of the whole database into a new file.
    fn backup(&self, target: &std::path::Path, timeout: Duration) -> Result<()> {
        let _ = (target, timeout);
        Err(crate::Error::new(
            409,
            "this storage backend has no built-in backup; use the database's own tools or export",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Unsupported;
    impl StorageMaintenance for Unsupported {}

    #[test]
    fn unsupported_maintenance_keeps_explicit_errors() {
        let backend = Unsupported;
        let timeout = Duration::from_secs(1);
        assert!(matches!(backend.export(&mut std::io::sink(), timeout), Err(e) if e.status == 409));
        assert!(
            matches!(backend.import(&mut std::io::empty(), timeout), Err(e) if e.status == 409)
        );
        assert!(
            matches!(backend.backup(std::path::Path::new("unused"), timeout), Err(e) if e.status == 409)
        );
    }
}
