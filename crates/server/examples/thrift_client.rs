//! Read-only Volo Thrift client; initialize the repository with the CLI first.
use geoledger_server::thrift_proto::{GeoLedgerClientBuilder, StatusRequest};
use volo_thrift::{MaybeException, codec::default::DefaultMakeCodec};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let address: std::net::SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7880".into())
        .parse()?;
    let client = GeoLedgerClientBuilder::new("geoledger")
        .address(address)
        .make_codec(DefaultMakeCodec::framed())
        .rpc_timeout(Some(std::time::Duration::from_secs(130)))
        .build();
    let authorization = std::env::var("GL_API_TOKEN")
        .ok()
        .map(|s| format!("Bearer {s}").into());
    match client
        .status(StatusRequest { limit: 20 }, authorization)
        .await?
    {
        MaybeException::Ok(reply) => println!("{}", reply.json),
        MaybeException::Exception(error) => {
            return Err(std::io::Error::other(format!("{error:?}")).into());
        }
    }
    Ok(())
}
