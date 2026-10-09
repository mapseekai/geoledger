use geoledger_client::{Client, FeatureQuery, Page};
use serde_json::json;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tokens: serde_json::Value =
        serde_json::from_slice(&std::fs::read(std::env::var("GL_TOKEN_FILE")?)?)?;
    let token = tokens[0]["token"].as_str().ok_or("token")?;
    let client = Client::connect(std::env::var("GL_ENDPOINT")?, token).await?;
    let project = client
        .create_project(&format!("rust-{}", uuid::Uuid::new_v4()))
        .await?;
    let dataset = client
        .create_dataset_with_dimension(&project.id, "places", "point", 3)
        .await?;
    let mut draft = client.create_workspace(&project.id).await?;
    let feature = json!({"type":"Feature","id":"one","properties":{"exact":18446744073709551615u64,"nested":{"geojson":"hello"},"detail_json":"plain"},"geometry":{"type":"Point","coordinates":[1,2,3]}});
    assert_eq!(draft.save(&dataset.id, feature.clone()).await?.version, 1);
    assert_eq!(draft.save(&dataset.id, feature.clone()).await?.version, 2);
    assert_eq!(
        draft
            .features(&dataset.id, FeatureQuery::default())
            .await?
            .features[0],
        feature
    );
    let missing = client.execute("save",json!({"project":project.id,"workspace":draft.id(),"expected_workspace_version":2,"edits":[{"dataset":dataset.id,"feature_id":"one"}]})).await.err().ok_or("missing feature accepted")?;
    assert_eq!(missing.code, "invalid_argument");
    draft.delete(&dataset.id, "one").await?;
    assert!(
        draft
            .features(&dataset.id, FeatureQuery::default())
            .await?
            .features
            .is_empty()
    );
    draft.save(&dataset.id, feature.clone()).await?;
    assert_eq!(draft.info().version, 4);
    let receipt = draft.publish("Rust SDK").await?;
    assert_eq!(receipt, draft.publish("Rust SDK").await?);
    assert_eq!(
        client
            .workspace_summary(&project.id, &receipt.workspace)
            .await?
            .total
            .added,
        1
    );
    assert_eq!(
        client
            .commit_summary(&project.id, receipt.revision)
            .await?
            .total
            .added,
        1
    );
    let rows = client
        .features(
            &project.id,
            &dataset.id,
            FeatureQuery {
                revision: Some(receipt.revision),
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(rows.features[0], feature);
    let history = client.history(&project.id, 0, None).await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].source_workspace, receipt.workspace);
    assert_eq!(history[0].source_base_revision, 0);
    assert_eq!(
        client
            .commit(&project.id, receipt.revision, Page::default())
            .await?
            .changes
            .len(),
        1
    );
    assert!(!client.audit(&project.id, 0, None).await?.events.is_empty());
    let mut changed = draft.pending_publication().ok_or("publication")?.clone();
    changed.message = "different".into();
    let error = client
        .publish(&changed)
        .await
        .err()
        .ok_or("mismatch accepted")?;
    assert_eq!(error.code, "conflict");
    assert!(!error.uncertain);
    assert_eq!(
        client
            .create_dataset_with_dimension(&project.id, "places", "point", 3)
            .await
            .err()
            .ok_or("duplicate")?
            .code,
        "conflict"
    );
    client
        .set_member(&project.id, "sdk-viewer", "viewer")
        .await?;
    let members = client.members(&project.id, Default::default()).await?;
    assert!(members.iter().any(|m| m.subject == "sdk-viewer"));
    client.remove_member(&project.id, "sdk-viewer").await?;
    assert_eq!(
        client.archive_project(&project.id, true).await?.state,
        "archived"
    );
    client.delete_project(&project.id, &project.name).await?;
    assert_eq!(
        client
            .project(&project.id)
            .await
            .err()
            .ok_or("deleted")?
            .code,
        "not_found"
    );
    println!(
        "Rust SDK: business API, exact numbers, automatic draft versions, publish retry, membership lifecycle OK"
    );
    Ok(())
}
