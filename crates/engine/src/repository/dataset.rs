use super::Row;
use crate::Result;

/// Dataset metadata and its transaction-local cache.
///
/// Schema operations are required even for backends without versioned schemas. Such
/// backends explicitly return unsupported, dynamic definitions; empty intent/history
/// and cleanup are valid no-ops, but nonempty schema replacement must be rejected.
/// A backend reporting `DatasetSchema::supported` must implement the full lifecycle
/// here and in `PublicationRepository`, on the same atomic transaction.
pub trait DatasetRepository: Send {
    /// Projection metadata lookup must not infer a schema by scanning feature payloads.
    fn schema_projection(
        &mut self,
        project: &str,
        dataset: &str,
        revision: i64,
    ) -> Result<crate::DatasetSchema>;
    /// Immutable property definitions at a published revision.
    fn schema_at(
        &mut self,
        project: &str,
        dataset: &str,
        revision: i64,
    ) -> Result<crate::DatasetSchema>;
    fn workspace_schemas(
        &mut self,
        project: &str,
        workspace: &str,
    ) -> Result<Vec<(String, crate::DatasetSchema)>>;
    /// Replaces the complete schema intent relative to the workspace base.
    fn replace_workspace_schemas(
        &mut self,
        project: &str,
        workspace: &str,
        definitions: &[(String, crate::DatasetSchema, crate::DatasetSchema)],
    ) -> Result<()>;
    fn published_schema_changes(
        &mut self,
        project: &str,
        revision: i64,
    ) -> Result<Vec<(String, crate::DatasetSchema, crate::DatasetSchema)>>;
    fn schema_changed_since(&mut self, project: &str, base: i64, current: i64) -> Result<bool>;
    fn clear_workspace_schemas(&mut self, project: &str, workspace: &str) -> Result<()>;

    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>>;

    #[allow(
        clippy::too_many_arguments,
        reason = "metadata and managed-table choice form one atomic dataset insertion"
    )]
    fn insert_dataset(
        &mut self,
        project: &str,
        dataset: &str,
        name: &str,
        geometry_type: &str,
        coordinate_dimension: i32,
        crs: &str,
        managed: bool,
    ) -> Result<()>;

    fn rename_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()>;

    fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
}
