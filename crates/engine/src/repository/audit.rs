use super::Row;
use crate::Result;

/// Project audit appends and key-only continuation probes in the current transaction.
pub trait AuditRepository: Send {
    /// Assign IDs in project commit order so audit cursors never skip a pending event.
    fn append_audit(
        &mut self,
        project: &str,
        subject: &str,
        action: &str,
        detail: &str,
    ) -> Result<()>;

    fn audit_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>>;

    fn audit_has_after(&mut self, project: &str, after: i64) -> Result<bool>;
}
