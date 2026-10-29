use super::Row;
use crate::Result;

/// Immutable commit history and change pages in the current snapshot.
pub trait HistoryRepository: Send {
    fn commit_summary(&mut self, project: &str, revision: i64) -> Result<Vec<Row>>;

    fn history_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>>;

    fn history_has_after(&mut self, project: &str, after: i64) -> Result<bool>;

    fn commit_exists(&mut self, project: &str, revision: i64) -> Result<Option<Row>>;

    fn commit_page(
        &mut self,
        project: &str,
        revision: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>>;

    fn commit_has_after(
        &mut self,
        project: &str,
        revision: i64,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<bool>;

    fn count_commit_changes(&mut self, project: &str, revision: i64) -> Result<Row>;
}
