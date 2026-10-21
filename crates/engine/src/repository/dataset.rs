use super::Row;
use crate::Result;

/// Dataset metadata and its transaction-local cache.
pub trait DatasetRepository: Send {
    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>>;

    fn insert_dataset(
        &mut self,
        project: &str,
        dataset: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: i32,
        crs: &str,
    ) -> Result<()>;

    fn rename_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()>;

    fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
}
