pub mod grpc;
pub mod http;
pub mod proto {
    tonic::include_proto!("spatial.version.v1");
}

use serde_json::Value;
use spatial_version::{Application, Command};
use spatial_version_core::{Error, Result};
use std::{net::SocketAddr, sync::Arc};
use subtle::ConstantTimeEq;

pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const DESCRIPTOR: &[u8] = tonic::include_file_descriptor_set!("spatial_version_descriptor");

#[derive(Clone)]
pub struct Service {
    application: Application,
    token: Option<Arc<str>>,
    permits: Arc<tokio::sync::Semaphore>,
}
impl Service {
    pub fn new(application: Application, token: Option<String>) -> Self {
        Self {
            application,
            token: token.map(Arc::from),
            permits: Arc::new(tokio::sync::Semaphore::new(8)),
        }
    }
    pub fn authorized(&self, authorization: Option<&str>) -> bool {
        let Some(token) = &self.token else {
            return true;
        };
        let Some(candidate) = authorization.and_then(|v| v.strip_prefix("Bearer ")) else {
            return false;
        };
        bool::from(candidate.as_bytes().ct_eq(token.as_bytes()))
    }
    pub async fn execute(&self, command: Command) -> Result<Value> {
        self.execute_with(command, |value| {
            encode_limited(&value, None)?;
            Ok(value)
        })
        .await
    }
    /// Serialize once on the blocking worker. Transports reuse this exact JSON.
    pub async fn execute_json(&self, command: Command) -> Result<String> {
        self.execute_with(command, |value| {
            let bytes = encode_limited(&value, Some(Vec::new()))?;
            String::from_utf8(bytes).map_err(|e| Error::storage_source("invalid JSON encoding", e))
        })
        .await
    }
    async fn execute_with<T: Send + 'static>(
        &self,
        command: Command,
        encode: impl FnOnce(Value) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let app = self.application.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            encode(app.execute(command)?)
        })
        .await
        .map_err(|e| Error::storage_source("operation worker failed", e))?
    }
}

struct LimitedJson {
    bytes: Option<Vec<u8>>,
    length: usize,
}
impl std::io::Write for LimitedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_RESPONSE_BYTES.saturating_sub(self.length) {
            return Err(std::io::Error::other("response size limit exceeded"));
        }
        self.length += bytes.len();
        if let Some(out) = &mut self.bytes {
            out.extend_from_slice(bytes);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn encode_limited(value: &Value, bytes: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let mut writer = LimitedJson { bytes, length: 0 };
    serde_json::to_writer(&mut writer, value).map_err(|_| {
        Error::Unsupported(
            "response exceeds 16 MiB; request fewer results or use the in-process API".into(),
        )
    })?;
    Ok(writer.bytes.unwrap_or_default())
}

pub struct ServerConfig {
    pub http: SocketAddr,
    pub grpc: SocketAddr,
    pub token: Option<String>,
}
impl ServerConfig {
    pub fn validate(&self) -> Result<()> {
        if self.token.as_ref().is_some_and(|t| t.len() < 24) {
            return Err(Error::Invalid(
                "SV_API_TOKEN must contain at least 24 bytes".into(),
            ));
        }
        if (!self.http.ip().is_loopback() || !self.grpc.ip().is_loopback()) && self.token.is_none()
        {
            return Err(Error::Invalid(
                "non-loopback listeners require SV_API_TOKEN; put TLS at a trusted reverse proxy"
                    .into(),
            ));
        }
        Ok(())
    }
}

pub async fn serve(application: Application, config: ServerConfig) -> Result<()> {
    config.validate()?;
    let http_listener = tokio::net::TcpListener::bind(config.http).await?;
    let grpc_listener = tokio::net::TcpListener::bind(config.grpc).await?;
    let service = Service::new(application, config.token);
    let reflection = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(DESCRIPTOR)
        .build_v1()
        .map_err(|e| Error::storage_source(e.to_string(), e))?;
    let rpc = grpc::server(service.clone());
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut http_shutdown = shutdown_rx.clone();
    let mut grpc_shutdown = shutdown_rx;
    let http =
        axum::serve(http_listener, http::router(service)).with_graceful_shutdown(async move {
            let _ = http_shutdown.changed().await;
        });
    let grpc = tonic::transport::Server::builder()
        .add_service(rpc)
        .add_service(reflection)
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(grpc_listener),
            async move {
                let _ = grpc_shutdown.changed().await;
            },
        );
    tracing::info!(http=%config.http,grpc=%config.grpc,"spatial-version listeners started");
    let mut http_task = tokio::spawn(async move { http.await });
    let mut grpc_task = tokio::spawn(grpc);
    tokio::select! {
        result=&mut http_task=>{
            let _=shutdown_tx.send(true);let _=grpc_task.await;
            result.map_err(|_|Error::Storage("HTTP task failed".into()))??;
        },
        result=&mut grpc_task=>{
            let _=shutdown_tx.send(true);let _=http_task.await;
            result.map_err(|_|Error::Storage("gRPC task failed".into()))?.map_err(|e|Error::storage_source(e.to_string(), e))?;
        },
        _=tokio::signal::ctrl_c()=>{
            let _=shutdown_tx.send(true);
            let (h,g)=tokio::join!(http_task,grpc_task);
            h.map_err(|_|Error::Storage("HTTP shutdown failed".into()))??;
            g.map_err(|_|Error::Storage("gRPC shutdown failed".into()))?.map_err(|e|Error::storage_source(e.to_string(), e))?;
        }
    }
    Ok(())
}

pub(crate) fn public_message(error: &Error) -> String {
    match error {
        Error::Backend { .. } | Error::Database(_) | Error::Storage(_) | Error::Io(_) => {
            tracing::error!(error=%error,"operation failed");
            "operation failed; inspect the server log for details".into()
        }
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    #[test]
    fn bounded_encoding_handles_escaping_and_rejects_before_exceeding_capacity() -> Result<()> {
        let value = serde_json::json!({"text":"a\n\"中"});
        let encoded = encode_limited(&value, Some(Vec::new()))?;
        assert_eq!(serde_json::from_slice::<Value>(&encoded)?, value);
        assert!(encode_limited(&Value::String("a".repeat(MAX_RESPONSE_BYTES)), None).is_err());
        assert!(
            encode_limited(
                &Value::String("a".repeat(MAX_RESPONSE_BYTES)),
                Some(Vec::new())
            )
            .is_err()
        );
        Ok(())
    }
}
