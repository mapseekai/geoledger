//! cargo run -p spatial-version --example library -- ./demo-repo
use spatial_version::{Application, Command};
use spatial_version_postgis::PostgisProvider;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repository = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./demo-repo".to_owned());
    let mut app = Application::new(repository);
    if let Ok(url) = std::env::var("SV_DATABASE_URL") {
        app = app.with_provider(Arc::new(PostgisProvider::new(url)));
    }
    // Initialize/import explicitly through the CLI first. This example is read-only.
    let status = app.execute(Command::Status { limit: 100 })?;
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}
