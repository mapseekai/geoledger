use super::Row;
use crate::Result;

/// Dataset metadata and its transaction-local cache.
pub trait DatasetRepository: Send {
    /// Projection metadata lookup must not infer a schema by scanning feature payloads.
    fn schema_projection(
        &mut self,
        _project: &str,
        _dataset: &str,
        _revision: i64,
    ) -> Result<crate::DatasetSchema> {
        Ok(crate::DatasetSchema {
            dynamic: true,
            ..Default::default()
        })
    }
    /// Immutable property definitions at a published revision.
    fn schema_at(
        &mut self,
        _project: &str,
        _dataset: &str,
        _revision: i64,
    ) -> Result<crate::DatasetSchema> {
        Ok(crate::DatasetSchema {
            dynamic: true,
            ..Default::default()
        })
    }
    fn workspace_schemas(
        &mut self,
        _project: &str,
        _workspace: &str,
    ) -> Result<Vec<(String, crate::DatasetSchema)>> {
        Ok(Vec::new())
    }
    /// Replaces the complete schema intent relative to the workspace base.
    fn replace_workspace_schemas(
        &mut self,
        _project: &str,
        _workspace: &str,
        definitions: &[(String, crate::DatasetSchema, crate::DatasetSchema)],
    ) -> Result<()> {
        if definitions.is_empty() {
            Ok(())
        } else {
            Err(crate::Error::new(
                400,
                "storage does not support versioned schemas",
            ))
        }
    }
    fn published_schema_changes(
        &mut self,
        _project: &str,
        _revision: i64,
    ) -> Result<Vec<(String, crate::DatasetSchema, crate::DatasetSchema)>> {
        Ok(Vec::new())
    }
    fn schema_changed_since(&mut self, _project: &str, _base: i64, _current: i64) -> Result<bool> {
        Ok(false)
    }
    fn clear_workspace_schemas(&mut self, _project: &str, _workspace: &str) -> Result<()> {
        Ok(())
    }

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
