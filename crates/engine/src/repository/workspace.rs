use super::Row;
use crate::Result;

pub struct DraftDelta {
    pub dataset: String,
    pub key: String,
    pub properties: Option<String>,
    pub geometry: Option<String>,
}
/// Workspace ownership, optimistic versions and draft edits in the current transaction.
pub trait WorkspaceRepository: Send {
    fn workspace_state(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        write: bool,
    ) -> Result<Option<Row>>;

    fn advance_workspace(&mut self, project: &str, workspace: &str, status: &str) -> Result<Row>;

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

    /// Apply a bounded batch, preserving any prior resolution pins on upsert.
    fn put_deltas(&mut self, project: &str, workspace: &str, deltas: &[DraftDelta]) -> Result<()>;

    fn invalidate_resolutions(&mut self, project: &str, workspace: &str) -> Result<()>;

    /// Resolution flag and feature at the workspace base; absent features have null fields.
    fn edit_base(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        base: i64,
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

    fn diff_page(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>>;

    /// Key-only continuation probes use this transaction's read snapshot and
    /// must not materialize unreturned feature properties, geometry or audit detail.
    fn diff_has_after(
        &mut self,
        project: &str,
        workspace: &str,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<bool>;

    fn mark_resolved(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        head: i64,
    ) -> Result<()>;

    fn clear_deltas(&mut self, project: &str, workspace: &str) -> Result<()>;

    fn advance_base(&mut self, project: &str, workspace: &str, revision: i64) -> Result<()>;
}
