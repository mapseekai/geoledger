//! A downstream, single-workspace backend fixture. It intentionally has no schema capability.
//! Unused operations fail explicitly; it is not a production in-memory backend.
use geoledger_engine::repository::*;
use geoledger_engine::{DatasetSchema, Error, ResponseReservation, Result};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const PROJECT: &str = "00000000-0000-4000-8000-000000000001";
pub const DATASET: &str = "00000000-0000-4000-8000-000000000002";
pub const WORKSPACE: &str = "00000000-0000-4000-8000-000000000003";
pub const SUBJECT: &str = "external-backend-user";

#[derive(Clone)]
pub struct Snapshot {
    pub head: i64,
    pub version: i64,
    pub status: String,
    pub published: Option<RawFeature>,
    pub draft: DraftState,
    pub schema_stages: usize,
    receipt: Option<(String, String, String)>,
    cached_conflicts: Option<i64>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            head: 0,
            version: 0,
            status: "open".into(),
            published: None,
            draft: DraftState::NoDraft,
            schema_stages: 0,
            receipt: None,
            cached_conflicts: None,
        }
    }
}

#[derive(Default)]
pub struct SchemaLessBackend {
    state: Arc<Mutex<Snapshot>>,
    modes: Mutex<Vec<bool>>,
}
impl SchemaLessBackend {
    pub fn snapshot(&self) -> Result<Snapshot> {
        self.state.lock().map(|v| v.clone()).map_err(poisoned)
    }
    pub fn modes(&self) -> Result<Vec<bool>> {
        self.modes.lock().map(|v| v.clone()).map_err(poisoned)
    }
}
fn poisoned<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::new(500, "fixture mutex poisoned")
}
fn unsupported_schema() -> DatasetSchema {
    DatasetSchema {
        dynamic: true,
        ..Default::default()
    }
}
fn cell(value: Option<&str>) -> Cell {
    value.map_or(Cell::Null, |v| Cell::Text(v.to_owned()))
}
fn feature_cells(value: Option<&RawFeature>) -> [Cell; 2] {
    [
        cell(value.map(|v| v.properties.as_str())),
        cell(value.and_then(|v| v.geometry.as_deref())),
    ]
}

impl StorageMaintenance for SchemaLessBackend {}
impl StorageBackend for SchemaLessBackend {
    fn name(&self) -> &'static str {
        "schema-less-fixture"
    }
    fn initialize(&self, _timeout: Duration) -> Result<()> {
        Ok(())
    }
    fn health(&self, _timeout: Duration) -> Result<()> {
        Ok(())
    }
    fn begin(&self, read_only: bool, _timeout: Duration) -> Result<Box<dyn RepositoryTransaction>> {
        self.modes.lock().map_err(poisoned)?.push(read_only);
        Ok(Box::new(Transaction {
            state: self.state.clone(),
            local: self.snapshot()?,
            read_only,
            limit: usize::MAX,
            reservation: None,
            staged: None,
        }))
    }
}

struct Transaction {
    state: Arc<Mutex<Snapshot>>,
    local: Snapshot,
    read_only: bool,
    limit: usize,
    reservation: Option<ResponseReservation>,
    staged: Option<Option<RawFeature>>,
}
impl Transaction {
    fn write(&self) -> Result<()> {
        if self.read_only {
            Err(Error::new(
                500,
                "write attempted in read-only fixture transaction",
            ))
        } else {
            Ok(())
        }
    }
    fn draft_value(&self) -> Option<&RawFeature> {
        match &self.local.draft {
            DraftState::NoDraft => self.local.published.as_ref(),
            DraftState::Deleted => None,
            DraftState::Value(value) => Some(value),
        }
    }
}
impl RepositoryTransaction for Transaction {
    fn commit(self: Box<Self>) -> Result<()> {
        if !self.read_only {
            *self.state.lock().map_err(poisoned)? = self.local;
        }
        Ok(())
    }
}
impl ResponseBudget for Transaction {
    fn set_response_limit(&mut self, bytes: usize) {
        self.limit = bytes;
    }
    fn response_limit(&self) -> usize {
        self.limit
    }
    fn set_response_reservation(&mut self, value: Option<ResponseReservation>) {
        self.reservation = value;
    }
    fn response_reservation(&self) -> Option<ResponseReservation> {
        self.reservation.clone()
    }
}

// Keep unrelated required signatures compile-checked while making accidental use fail.
macro_rules! unused_methods {
    ($(fn $name:ident(&mut self $(, $arg:ident: $ty:ty)* $(,)?) -> $result:ty;)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) -> $result {
            $(let _ = $arg;)*
            Err(Error::new(500, concat!("unused fixture operation: ", stringify!($name))))
        })*
    };
}

impl DatasetRepository for Transaction {
    fn schema_projection(
        &mut self,
        _project: &str,
        _dataset: &str,
        _revision: i64,
    ) -> Result<DatasetSchema> {
        Ok(unsupported_schema())
    }
    fn schema_at(
        &mut self,
        _project: &str,
        _dataset: &str,
        _revision: i64,
    ) -> Result<DatasetSchema> {
        Ok(unsupported_schema())
    }
    fn workspace_schemas(
        &mut self,
        _project: &str,
        _workspace: &str,
    ) -> Result<Vec<(String, DatasetSchema)>> {
        Ok(Vec::new())
    }
    fn replace_workspace_schemas(
        &mut self,
        _project: &str,
        _workspace: &str,
        definitions: &[(String, DatasetSchema, DatasetSchema)],
    ) -> Result<()> {
        if definitions.is_empty() {
            Ok(())
        } else {
            Err(Error::new(
                400,
                "storage does not support versioned schemas",
            ))
        }
    }
    fn published_schema_changes(
        &mut self,
        _project: &str,
        _revision: i64,
    ) -> Result<Vec<(String, DatasetSchema, DatasetSchema)>> {
        Ok(Vec::new())
    }
    fn schema_changed_since(&mut self, _project: &str, _base: i64, _current: i64) -> Result<bool> {
        Ok(false)
    }
    fn clear_workspace_schemas(&mut self, _project: &str, _workspace: &str) -> Result<()> {
        Ok(())
    }
    fn dataset_exists(&mut self, project: &str, dataset: &str) -> Result<Option<Row>> {
        Ok((project == PROJECT && dataset == DATASET).then(|| {
            Row::new(vec![
                Cell::Text("point".into()),
                Cell::Text("fixture".into()),
                Cell::Integer(2),
                Cell::Null,
                Cell::Text("EPSG:4326".into()),
            ])
        }))
    }
    unused_methods! {
        fn insert_dataset(&mut self, project: &str, dataset: &str, name: &str, geometry_type: &str, coordinate_dimension: i32, crs: &str, managed: bool) -> Result<()>;
        fn rename_dataset(&mut self, project: &str, dataset: &str, name: &str) -> Result<()>;
        fn list_datasets(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
    }
}

impl WorkspaceRepository for Transaction {
    fn workspace_state(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        _write: bool,
    ) -> Result<Option<Row>> {
        Ok(
            (project == PROJECT && workspace == WORKSPACE && subject == SUBJECT).then(|| {
                Row::new(vec![
                    Cell::Integer(0),
                    Cell::Integer(self.local.version),
                    Cell::Text(self.local.status.clone()),
                ])
            }),
        )
    }
    fn workspace_publication_revision(
        &mut self,
        _project: &str,
        _workspace: &str,
    ) -> Result<Option<i64>> {
        Ok((self.local.status == "published").then_some(self.local.head))
    }
    fn advance_workspace(&mut self, _project: &str, _workspace: &str, status: &str) -> Result<Row> {
        self.write()?;
        self.local.version += 1;
        self.local.status = status.to_owned();
        Ok(Row::new(vec![Cell::Integer(self.local.version)]))
    }
    fn put_deltas(
        &mut self,
        _project: &str,
        _workspace: &str,
        deltas: &[DraftDelta],
    ) -> Result<()> {
        self.write()?;
        for delta in deltas {
            if delta.dataset != DATASET || delta.key != "one" {
                return Err(Error::new(500, "fixture only supports one feature"));
            }
            self.local.draft = match &delta.properties {
                None => DraftState::Deleted,
                Some(properties) => DraftState::Value(RawFeature {
                    properties: properties.clone(),
                    geometry: delta.geometry.clone(),
                }),
            };
        }
        Ok(())
    }
    fn invalidate_resolutions(&mut self, _project: &str, _workspace: &str) -> Result<()> {
        self.write()
    }
    fn edit_base(
        &mut self,
        _project: &str,
        _workspace: &str,
        _dataset: &str,
        _key: &str,
        _base: i64,
    ) -> Result<EditContext> {
        Ok(EditContext {
            resolved: false,
            base: self.local.published.clone(),
            draft: self.local.draft.clone(),
        })
    }
    fn remove_delta(
        &mut self,
        _project: &str,
        _workspace: &str,
        _dataset: &str,
        _key: &str,
    ) -> Result<()> {
        self.write()?;
        self.local.draft = DraftState::NoDraft;
        Ok(())
    }
    fn count_deltas(&mut self, _project: &str, _workspace: &str) -> Result<Row> {
        Ok(Row::new(vec![Cell::Integer(i64::from(!matches!(
            self.local.draft,
            DraftState::NoDraft
        )))]))
    }
    fn clear_deltas(&mut self, _project: &str, _workspace: &str) -> Result<()> {
        self.write()?;
        self.local.draft = DraftState::NoDraft;
        Ok(())
    }
    unused_methods! {
        fn insert_workspace(&mut self, project: &str, workspace: &str, subject: &str, base: i64) -> Result<()>;
        fn list_workspaces(&mut self, project: &str, subject: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
        fn workspace_summary(&mut self, project: &str, workspace: &str, base: i64) -> Result<Vec<Row>>;
        fn diff_page(&mut self, project: &str, workspace: &str, base: i64, after_dataset: &str, after_key: &str, limit: i64) -> Result<Vec<Row>>;
        fn diff_has_after(&mut self, project: &str, workspace: &str, after_dataset: &str, after_key: &str) -> Result<bool>;
        fn mark_resolved(&mut self, project: &str, workspace: &str, dataset: &str, key: &str, head: i64) -> Result<()>;
        fn advance_base(&mut self, project: &str, workspace: &str, revision: i64) -> Result<()>;
    }
}

impl PublicationRepository for Transaction {
    fn stage_schema_merge(
        &mut self,
        _project: &str,
        _workspace: &str,
        _current: i64,
    ) -> Result<usize> {
        self.write()?;
        self.local.schema_stages += 1;
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
    fn cached_conflict_count(
        &mut self,
        _project: &str,
        _workspace: &str,
        _base: i64,
        _version: i64,
        _head: i64,
    ) -> Result<Option<i64>> {
        Ok(self.local.cached_conflicts)
    }
    fn reset_conflict_cache(
        &mut self,
        _project: &str,
        _workspace: &str,
        _base: i64,
        _version: i64,
        _head: i64,
    ) -> Result<()> {
        self.write()?;
        self.local.cached_conflicts = None;
        Ok(())
    }
    fn cache_conflict_keys(
        &mut self,
        _project: &str,
        _workspace: &str,
        _keys: &[(String, String)],
    ) -> Result<()> {
        self.write()
    }
    fn complete_conflict_cache(
        &mut self,
        _project: &str,
        _workspace: &str,
        total: i64,
    ) -> Result<()> {
        self.write()?;
        self.local.cached_conflicts = Some(total);
        Ok(())
    }
    fn cached_conflict_page(&mut self, _query: &ConflictQuery<'_>) -> Result<Vec<Row>> {
        Ok(Vec::new())
    }
    fn begin_merge(&mut self) -> Result<()> {
        self.write()?;
        self.staged = None;
        Ok(())
    }
    fn merge_page(
        &mut self,
        _project: &str,
        _workspace: &str,
        _base: i64,
        _current: i64,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<Vec<Row>> {
        // The initial application cursor is (nil UUID, ""), not an empty dataset.
        // Apply the same strict tuple continuation as a real repository page.
        if (DATASET, "one") <= (after_dataset, after_key)
            || matches!(self.local.draft, DraftState::NoDraft)
        {
            return Ok(Vec::new());
        }
        let mut values = vec![Cell::Text(DATASET.into()), Cell::Text("one".into())];
        values.extend(feature_cells(self.draft_value()));
        values.extend([Cell::Null, Cell::Bool(false)]);
        values.extend(feature_cells(self.local.published.as_ref()));
        values.extend(feature_cells(self.local.published.as_ref()));
        Ok(vec![Row::new(values)])
    }
    fn stage_merge(
        &mut self,
        _dataset: &str,
        _key: &str,
        _before: &Option<String>,
        after: &Option<String>,
    ) -> Result<()> {
        self.write()?;
        self.staged = Some(
            after
                .as_ref()
                .map(|text| {
                    let value = geoledger_engine::parse_json(text.as_bytes())?;
                    Ok::<RawFeature, Error>(RawFeature {
                        properties: value["properties"].to_string(),
                        geometry: value["geometry"].as_str().map(str::to_owned),
                    })
                })
                .transpose()?,
        );
        Ok(())
    }
    fn publication_receipt(
        &mut self,
        _project: &str,
        _subject: &str,
        request_id: &str,
    ) -> Result<Option<Row>> {
        Ok(self
            .local
            .receipt
            .as_ref()
            .filter(|(id, _, _)| id == request_id)
            .map(|(_, payload, result)| {
                Row::new(vec![
                    Cell::Text(payload.clone()),
                    Cell::Text(result.clone()),
                ])
            }))
    }
    fn append_commit(
        &mut self,
        _project: &str,
        _revision: i64,
        _workspace: &str,
        _subject: &str,
        _message: &str,
    ) -> Result<()> {
        self.write()
    }
    fn append_changes(&mut self, _project: &str, _revision: i64) -> Result<()> {
        self.write()
    }
    fn close_history(&mut self, _project: &str, _revision: i64) -> Result<()> {
        self.write()
    }
    fn append_history(&mut self, _project: &str, _revision: i64) -> Result<()> {
        self.write()?;
        if let Some(value) = &self.staged {
            self.local.published = value.clone();
        }
        Ok(())
    }
    fn advance_head(&mut self, _project: &str, revision: i64) -> Result<()> {
        self.write()?;
        self.local.head = revision;
        Ok(())
    }
    fn save_receipt(
        &mut self,
        _project: &str,
        _subject: &str,
        request_id: &str,
        payload: &str,
        result: &str,
    ) -> Result<()> {
        self.write()?;
        self.local.receipt = Some((request_id.into(), payload.into(), result.into()));
        Ok(())
    }
    unused_methods! {
        fn stage_resolution(&mut self, dataset: &str, key: &str, after: &Option<String>) -> Result<()>;
        fn replace_deltas_with_merge(&mut self, project: &str, workspace: &str) -> Result<()>;
        fn restore_deltas(&mut self, project: &str, revision: i64, workspace: &str) -> Result<()>;
    }
}

impl ProjectRepository for Transaction {
    fn member_role(&mut self, project: &str, subject: &str) -> Result<Option<Row>> {
        Ok((project == PROJECT && subject == SUBJECT).then(|| {
            Row::new(vec![
                Cell::Text("owner".into()),
                Cell::Text("active".into()),
            ])
        }))
    }
    fn project_head(&mut self, project: &str, _lock: bool) -> Result<Option<Row>> {
        Ok((project == PROJECT).then(|| Row::new(vec![Cell::Integer(self.local.head)])))
    }
    unused_methods! {
        fn insert_project(&mut self, project: &str, name: &str) -> Result<()>;
        fn insert_owner(&mut self, project: &str, subject: &str) -> Result<()>;
        fn list_projects(&mut self, subject: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
        fn project_info(&mut self, project: &str) -> Result<Row>;
        fn live_project(&mut self, project: &str, lock: bool) -> Result<Option<Row>>;
        fn list_members(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>>;
        fn remove_member(&mut self, project: &str, subject: &str) -> Result<()>;
        fn set_project_state(&mut self, project: &str, state: &str) -> Result<()>;
        fn owned_projects(&mut self, subject: &str) -> Result<Row>;
        fn ensure_identity(&mut self, project: &str, subject: &str) -> Result<()>;
        fn owner_summary(&mut self, project: &str, subject: &str) -> Result<Row>;
        fn set_member(&mut self, project: &str, subject: &str, role: &str) -> Result<()>;
        fn rename_project(&mut self, project: &str, name: &str) -> Result<()>;
        fn purge_data(&mut self, project: &str, dataset: Option<&str>) -> Result<()>;
    }
}
impl FeatureRepository for Transaction {
    fn feature_exists(&mut self, query: &FeatureQuery) -> Result<bool> {
        let value = if query.workspace.is_some() && self.local.status == "open" {
            self.draft_value()
        } else {
            self.local.published.as_ref()
        };
        Ok(query.after.as_str() < "one" && value.is_some())
    }

    fn feature_page(&mut self, query: &FeatureQuery) -> Result<Vec<Row>> {
        let value = if query.workspace.is_some() && self.local.status == "open" {
            self.draft_value()
        } else {
            self.local.published.as_ref()
        };
        if query.after.as_str() >= "one" {
            return Ok(Vec::new());
        }
        Ok(value
            .map(|value| {
                let mut cells = vec![Cell::Text("one".into())];
                cells.extend(feature_cells(Some(value)));
                Row::new(cells)
            })
            .into_iter()
            .collect())
    }
    fn feature_at(
        &mut self,
        _project: &str,
        _dataset: &str,
        _key: &str,
        _revision: i64,
    ) -> Result<Option<Row>> {
        Ok(self
            .local
            .published
            .as_ref()
            .map(|value| Row::new(feature_cells(Some(value)).into())))
    }
}
impl AuditRepository for Transaction {
    fn append_audit(
        &mut self,
        _project: &str,
        _subject: &str,
        _action: &str,
        _detail: &str,
    ) -> Result<()> {
        self.write()
    }
    unused_methods! {
        fn audit_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>>;
        fn audit_has_after(&mut self, project: &str, after: i64) -> Result<bool>;
    }
}
impl HistoryRepository for Transaction {
    unused_methods! {
        fn commit_summary(&mut self, project: &str, revision: i64) -> Result<Vec<Row>>;
        fn history_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>>;
        fn history_has_after(&mut self, project: &str, after: i64) -> Result<bool>;
        fn commit_exists(&mut self, project: &str, revision: i64) -> Result<Option<Row>>;
        fn commit_page(&mut self, project: &str, revision: i64, after_dataset: &str, after_key: &str, limit: i64) -> Result<Vec<Row>>;
        fn commit_has_after(&mut self, project: &str, revision: i64, after_dataset: &str, after_key: &str) -> Result<bool>;
        fn count_commit_changes(&mut self, project: &str, revision: i64) -> Result<Row>;
    }
}
impl CredentialRepository for Transaction {
    unused_methods! {
        fn lock_credentials(&mut self) -> Result<()>;
        fn auth_user(&mut self, id: &str) -> Result<Option<geoledger_engine::authentication::UserRecord>>;
        fn auth_username(&mut self, username: &str) -> Result<Option<geoledger_engine::authentication::UserRecord>>;
        fn auth_users(&mut self, after: &str, limit: i64, status: &str) -> Result<Vec<geoledger_engine::User>>;
        fn auth_insert_user(&mut self, user: &geoledger_engine::authentication::UserRecord) -> Result<()>;
        fn auth_update_user(&mut self, user: &geoledger_engine::authentication::UserRecord) -> Result<()>;
        fn auth_admin_count(&mut self) -> Result<i64>;
        fn auth_membership_target(&mut self, id: &str) -> Result<bool>;
        fn auth_user_count(&mut self) -> Result<i64>;
        fn auth_key(&mut self, id: &str) -> Result<Option<geoledger_engine::ApiKey>>;
        fn auth_key_digest(&mut self, digest: &str, now: &str) -> Result<Option<geoledger_engine::ApiKey>>;
        fn auth_keys(&mut self, user: &str, after: &str, limit: i64) -> Result<Vec<geoledger_engine::ApiKey>>;
        fn auth_insert_key(&mut self, key: &geoledger_engine::ApiKey, digest: &str) -> Result<()>;
        fn auth_update_key(&mut self, key: &geoledger_engine::ApiKey) -> Result<()>;
        fn auth_delete_key(&mut self, id: &str) -> Result<()>;
        fn auth_touch_key(&mut self, id: &str, now: &str, before: &str) -> Result<()>;
        fn auth_session(&mut self, digest: &str) -> Result<Option<geoledger_engine::authentication::SessionRecord>>;
        fn auth_insert_session(&mut self, session: &geoledger_engine::authentication::SessionRecord, digest: &str) -> Result<()>;
        fn auth_touch_session(&mut self, id: &str, now: &str, before: &str) -> Result<()>;
        fn auth_delete_session(&mut self, id: &str) -> Result<()>;
        fn auth_invalidate_user(&mut self, id: &str, now: &str, keys: bool) -> Result<()>;
        fn auth_audit(&mut self, actor: &str, action: &str, resource: &str, now: &str) -> Result<()>;
    }
}
impl PostgisTables for Transaction {}
