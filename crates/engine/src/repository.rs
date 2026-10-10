//! Backend extension contract. All mutations, receipts and audit writes share one atomic transaction.
//! Implementations must roll back on Drop, serialize competing project publications, preserve
//! read snapshots, bound query pages, and honour the supplied end-to-end deadline.
use crate::Result;
pub use crate::session::{Cell, Decode, Row};
use std::time::Duration;
/// A row follows the ordered field schema documented in docs/storage.md.
/// New backends construct rows using Row::new; no database-specific value escapes.
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn initialize(&self, timeout: Duration) -> Result<()>;
    fn health(&self, timeout: Duration) -> Result<()>;
    fn begin(&self, read_only: bool, timeout: Duration) -> Result<Box<dyn RepositoryTransaction>>;
    /// Connection pool occupancy for metrics; backends without a pool return None.
    fn pool_stats(&self) -> Option<crate::PoolStats> {
        None
    }
    /// Write every table to `out` from one consistent snapshot (see `DataSummary`).
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
#[derive(Clone)]
pub struct FeatureQuery {
    pub project: String,
    pub dataset: String,
    pub revision: i64,
    pub workspace: Option<String>,
    pub after: String,
    pub feature_id: Option<String>,
    pub bbox: Option<[f64; 4]>,
    pub limit: i64,
}
/// All methods are scoped by a validated project; authorization remains in Application.
/// begin_merge/stage_merge are transaction-local staging, never durable publication.
pub trait RepositoryTransaction: Send {
    /// (role, project state) for an active member of a project that is not deleted.
    fn member_role(&mut self, project: &str, subject: &str) -> Result<Option<Row>>;
    fn project_head(&mut self, project: &str, lock: bool) -> Result<Option<Row>>;
    fn workspace_state(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        write: bool,
    ) -> Result<Option<Row>>;
    fn feature_page(&mut self, query: &FeatureQuery) -> Result<Vec<Row>>;
    fn feature_at(
        &mut self,
        project: &str,
        dataset: &str,
        key: &str,
        revision: i64,
    ) -> Result<Option<Row>>;
    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>>;
    /// Assign IDs in project commit order so audit cursors never skip a pending event.
    fn append_audit(
        &mut self,
        project: &str,
        subject: &str,
        action: &str,
        detail: &str,
    ) -> Result<()>;
    fn advance_workspace(&mut self, project: &str, workspace: &str, status: &str) -> Result<Row>;
    fn insert_project(&mut self, project: &str, name: &str) -> Result<()>;
    fn insert_owner(&mut self, project: &str, subject: &str) -> Result<()>;
    /// (project, name, head, state, role) for active memberships of projects that are not deleted.
    fn list_projects(&mut self, subject: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
    /// (name, head, state).
    fn project_info(&mut self, project: &str) -> Result<Row>;
    /// (name, head, state) of a project that is not deleted; `lock` serializes lifecycle changes.
    fn live_project(&mut self, project: &str, lock: bool) -> Result<Option<Row>>;
    /// (subject, role) of active members ordered by subject.
    fn list_members(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
    /// Deactivate a membership; history and audit rows keep referencing it.
    fn remove_member(&mut self, project: &str, subject: &str) -> Result<()>;
    /// One of active, archived, deleted.
    fn set_project_state(&mut self, project: &str, state: &str) -> Result<()>;
    /// Lock creation for this subject through commit and count their active owned projects.
    fn owned_projects(&mut self, subject: &str) -> Result<Row>;
    /// Ensure an (inactive, if new) member row exists so audit events can reference `subject`.
    fn ensure_identity(&mut self, project: &str, subject: &str) -> Result<()>;
    fn owner_summary(&mut self, project: &str, subject: &str) -> Result<Row>;
    fn set_member(&mut self, project: &str, subject: &str, role: &str) -> Result<()>;
    fn insert_dataset(
        &mut self,
        project: &str,
        dataset: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: i32,
    ) -> Result<()>;
    /// Validate and lock a business table; returns (geometry family, dimension).
    fn inspect_postgis_table(&mut self, source: &crate::PostgisTable) -> Result<Row>;
    /// Source rows (id, properties JSON, geometry JSON), ordered by id with C collation.
    fn postgis_table_page(&mut self, source: &crate::PostgisTable, after: &str)
    -> Result<Vec<Row>>;
    fn bind_postgis_table(
        &mut self,
        project: &str,
        dataset: &str,
        source: &crate::PostgisTable,
    ) -> Result<()>;
    fn rename_project(&mut self, project: &str, name: &str) -> Result<()>;
    fn rename_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()>;
    /// Purge the selected dataset, or all project data. Caller holds the project lock.
    /// Shared commits/workspaces retain other datasets; audit records remain immutable.
    fn purge_data(&mut self, project: &str, dataset: Option<&str>) -> Result<()>;
    fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
    fn insert_workspace(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        base: i64,
    ) -> Result<()>;
    fn list_workspaces(
        &mut self,
        project: &str,
        subject: &str,
        after: &str,
        limit: i64,
    ) -> Result<Vec<Row>>;
    fn put_delta(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        properties: &Option<String>,
        geometry: &Option<&str>,
    ) -> Result<()>;
    fn invalidate_resolutions(&mut self, project: &str, workspace: &str) -> Result<()>;
    fn has_resolution(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
    ) -> Result<Row>;
    fn remove_delta(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
    ) -> Result<()>;
    fn count_deltas(&mut self, project: &str, workspace: &str) -> Result<Row>;
    fn workspace_summary(&mut self, project: &str, workspace: &str, base: i64) -> Result<Vec<Row>>;
    fn commit_summary(&mut self, project: &str, revision: i64) -> Result<Vec<Row>>;
    fn diff_page(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>>;
    fn history_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>>;
    fn commit_exists(&mut self, project: &str, revision: i64) -> Result<Option<Row>>;
    fn commit_page(
        &mut self,
        project: &str,
        revision: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>>;
    fn audit_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>>;
    fn begin_merge(&mut self) -> Result<()>;
    fn merge_page(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        current: i64,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<Vec<Row>>;
    fn stage_merge(
        &mut self,
        dataset: &str,
        key: &str,
        before: &Option<String>,
        after: &Option<String>,
    ) -> Result<()>;
    fn publication_receipt(
        &mut self,
        project: &str,
        subject: &str,
        request_id: &str,
    ) -> Result<Option<Row>>;
    fn append_commit(
        &mut self,
        project: &str,
        revision: i64,
        workspace: &str,
        subject: &str,
        message: &str,
    ) -> Result<()>;
    fn append_changes(&mut self, project: &str, revision: i64) -> Result<()>;
    fn close_history(&mut self, project: &str, revision: i64) -> Result<()>;
    fn append_history(&mut self, project: &str, revision: i64) -> Result<()>;
    fn advance_head(&mut self, project: &str, revision: i64) -> Result<()>;
    fn save_receipt(
        &mut self,
        project: &str,
        subject: &str,
        request_id: &str,
        payload: &str,
        result: &str,
    ) -> Result<()>;
    fn mark_resolved(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        head: i64,
    ) -> Result<()>;
    fn stage_resolution(&mut self, dataset: &str, key: &str, after: &Option<String>) -> Result<()>;
    fn clear_deltas(&mut self, project: &str, workspace: &str) -> Result<()>;
    fn replace_deltas_with_merge(&mut self, project: &str, workspace: &str) -> Result<()>;
    fn advance_base(&mut self, project: &str, workspace: &str, revision: i64) -> Result<()>;
    fn count_commit_changes(&mut self, project: &str, revision: i64) -> Result<Row>;
    fn restore_deltas(&mut self, project: &str, revision: i64, workspace: &str) -> Result<()>;
    fn commit(self: Box<Self>) -> Result<()>;
}
