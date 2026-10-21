use super::Row;
use crate::Result;

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
/// Feature reads share the transaction snapshot, filters and response budget.
pub trait FeatureRepository: Send {
    fn feature_page(&mut self, query: &FeatureQuery) -> Result<Vec<Row>>;

    /// Check the same snapshot, cursor and filters for a match, ignoring the page limit.
    /// Backends should override this to avoid loading feature payloads for lookahead.
    fn feature_exists(&mut self, query: &FeatureQuery) -> Result<bool> {
        let mut query = query.clone();
        query.limit = 1;
        Ok(!self.feature_page(&query)?.is_empty())
    }

    fn feature_at(
        &mut self,
        project: &str,
        dataset: &str,
        key: &str,
        revision: i64,
    ) -> Result<Option<Row>>;
}
