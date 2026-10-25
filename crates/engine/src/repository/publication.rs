use super::Row;
use crate::Result;

#[derive(Clone, Copy)]
pub struct ConflictQuery<'a> {
    pub project: &'a str,
    pub workspace: &'a str,
    pub base: i64,
    pub head: i64,
    pub after_dataset: &'a str,
    pub after_key: &'a str,
    pub limit: i64,
}
/// Transaction-local merge/conflict state, immutable publications and original-request receipts.
pub trait PublicationRepository: Send {
    /// Materialize schema projections into the existing bounded merge stage.
    fn stage_schema_merge(
        &mut self,
        _project: &str,
        _workspace: &str,
        _current: i64,
    ) -> Result<usize> {
        Ok(0)
    }
    fn restore_workspace_schemas(
        &mut self,
        _project: &str,
        _workspace: &str,
        _revision: i64,
        _current: i64,
    ) -> Result<()> {
        Ok(())
    }

    /// Transactional conflict index for exactly one workspace/version/HEAD tuple.
    /// Its rows are derived data and must not grant access without application authorization.
    fn cached_conflict_count(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        version: i64,
        head: i64,
    ) -> Result<Option<i64>>;

    fn reset_conflict_cache(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        version: i64,
        head: i64,
    ) -> Result<()>;

    fn cache_conflict_keys(
        &mut self,
        project: &str,
        workspace: &str,
        keys: &[(String, String)],
    ) -> Result<()>;

    fn complete_conflict_cache(&mut self, project: &str, workspace: &str, total: i64)
    -> Result<()>;

    fn cached_conflict_page(&mut self, query: &ConflictQuery<'_>) -> Result<Vec<Row>>;

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

    fn stage_resolution(&mut self, dataset: &str, key: &str, after: &Option<String>) -> Result<()>;

    fn replace_deltas_with_merge(&mut self, project: &str, workspace: &str) -> Result<()>;

    fn restore_deltas(&mut self, project: &str, revision: i64, workspace: &str) -> Result<()>;
}
