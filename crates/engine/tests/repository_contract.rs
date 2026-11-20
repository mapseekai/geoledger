//! Exercise the storage extension contract from a real downstream test crate.
#[path = "support/schema_less_backend.rs"]
mod schema_less_backend;

use geoledger_engine::repository::{DraftState, StorageBackend};
use geoledger_engine::{Application, DatasetSchema};
use schema_less_backend::{DATASET, PROJECT, SUBJECT, SchemaLessBackend, WORKSPACE};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn external_backend_explicitly_rejects_schema_edits_and_publishes_ordinary_features() -> TestResult
{
    let backend = Arc::new(SchemaLessBackend::default());
    let app = Application::with_backend(backend.clone());
    let mut transaction = backend.begin(false, Duration::from_secs(1))?;
    let schema = transaction.schema_at(PROJECT, DATASET, 0)?;
    assert!(!schema.supported);
    assert!(schema.dynamic);
    assert_eq!(transaction.schema_projection(PROJECT, DATASET, 0)?, schema);
    assert!(
        transaction
            .workspace_schemas(PROJECT, WORKSPACE)?
            .is_empty()
    );
    assert!(transaction.published_schema_changes(PROJECT, 0)?.is_empty());
    assert!(!transaction.schema_changed_since(PROJECT, 0, 1)?);
    transaction.replace_workspace_schemas(PROJECT, WORKSPACE, &[])?;
    let result = transaction.replace_workspace_schemas(
        PROJECT,
        WORKSPACE,
        &[(DATASET.into(), schema, DatasetSchema::default())],
    );
    assert!(matches!(result, Err(error) if error.status == 400));
    transaction.restore_workspace_schemas(PROJECT, WORKSPACE, 0, 1)?;
    transaction.clear_workspace_schemas(PROJECT, WORKSPACE)?;
    drop(transaction);

    let query = app.execute(
        SUBJECT,
        "schema",
        json!({"project":PROJECT,"dataset":DATASET}),
    );
    assert!(matches!(query, Err(error) if error.status == 400));
    assert_eq!(backend.modes()?.last(), Some(&true));

    let rejected = app.execute(
        SUBJECT,
        "save",
        json!({
            "project":PROJECT,"workspace":WORKSPACE,"expected_workspace_version":0,
            "schema_edits":[{"dataset":DATASET,"add":[{"name":"height","type":"number"}]}]
        }),
    );
    assert!(matches!(rejected, Err(error) if error.status == 400));
    assert_eq!(backend.snapshot()?.version, 0);
    assert_eq!(backend.snapshot()?.draft, DraftState::NoDraft);

    let properties = json!({"exact":18446744073709551615_u64,"nested":{"name":"ordinary JSON"}});
    let saved = app.execute(SUBJECT, "save", json!({
        "project":PROJECT,"workspace":WORKSPACE,"expected_workspace_version":0,"schema_edits":[],
        "edits":[{"dataset":DATASET,"feature_id":"one","feature":{
            "type":"Feature","id":"one","properties":properties,"geometry":null
        }}]
    }))?;
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["changes"], 1);
    let DraftState::Value(raw) = backend.snapshot()?.draft else {
        return Err("external backend did not retain the raw draft".into());
    };
    assert_eq!(
        geoledger_engine::parse_json(raw.properties.as_bytes())?,
        properties
    );

    // This read-looking operation writes its derived conflict cache. The fixture
    // refuses every write in a read-only transaction, so this checks actual admission.
    let conflicts = app.execute(
        SUBJECT,
        "conflicts",
        json!({"project":PROJECT,"workspace":WORKSPACE}),
    )?;
    assert_eq!(conflicts["total"], 0);
    assert_eq!(backend.modes()?.last(), Some(&false));
    app.execute(
        SUBJECT,
        "get_workspace",
        json!({"project":PROJECT,"workspace":WORKSPACE}),
    )?;
    assert_eq!(backend.modes()?.last(), Some(&true));

    let request = json!({"project":PROJECT,"workspace":WORKSPACE,"expected_workspace_version":1,"request_id":"00000000-0000-4000-8000-000000000004","message":"ordinary publication"});
    let receipt = app.execute(SUBJECT, "publish", request.clone())?;
    assert_eq!(receipt["revision"], 1);
    assert_eq!(receipt["changes"], 1);
    assert_eq!(app.execute(SUBJECT, "publish", request)?, receipt);
    let snapshot = backend.snapshot()?;
    assert_eq!(snapshot.head, 1);
    assert_eq!(snapshot.status, "published");
    assert_eq!(snapshot.draft, DraftState::NoDraft);
    assert_eq!(snapshot.schema_stages, 1);
    let published = snapshot.published.ok_or("missing published feature")?;
    assert_eq!(
        geoledger_engine::parse_json(published.properties.as_bytes())?,
        properties
    );
    Ok(())
}

#[test]
fn external_backend_empty_publication_keeps_schema_staging_a_legal_noop() -> TestResult {
    let backend = Arc::new(SchemaLessBackend::default());
    let app = Application::with_backend(backend.clone());
    let receipt = app.execute(
        SUBJECT,
        "publish",
        json!({
            "project":PROJECT,"workspace":WORKSPACE,"expected_workspace_version":0,
            "request_id":"00000000-0000-4000-8000-000000000005","message":"empty publication"
        }),
    )?;
    assert_eq!(receipt["revision"], 1);
    assert_eq!(receipt["changes"], 0);
    assert_eq!(backend.snapshot()?.schema_stages, 1);
    assert_eq!(backend.snapshot()?.published, None);
    Ok(())
}
