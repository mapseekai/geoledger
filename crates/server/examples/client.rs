//! Read status from a running server; SV_API_TOKEN is optional on loopback.
use spatial_version_server::proto::{StatusRequest, spatial_version_client::SpatialVersionClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:7879".into());
    let mut client = SpatialVersionClient::connect(address).await?;
    let mut request = tonic::Request::new(StatusRequest { limit: 100 });
    if let Ok(token) = std::env::var("SV_API_TOKEN") {
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse()?);
    }
    let response = client.status(request).await?.into_inner();
    println!("{}", response.json);
    Ok(())
}
