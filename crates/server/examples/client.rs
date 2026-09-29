//! Read status from a running server; GL_API_TOKEN is optional on loopback.
use geoledger_server::proto::{StatusRequest, geo_ledger_client::GeoLedgerClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:7879".into());
    let mut client = GeoLedgerClient::connect(address).await?;
    let mut request = tonic::Request::new(StatusRequest { limit: 100 });
    if let Ok(token) = std::env::var("GL_API_TOKEN") {
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse()?);
    }
    let response = client.status(request).await?.into_inner();
    println!("{}", response.json);
    Ok(())
}
