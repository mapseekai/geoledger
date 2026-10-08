//! One server, shared authentication and bounded application execution for HTTP and gRPC.
mod auth;
mod http;
mod rpc;
mod telemetry;
mod tokens;
pub use auth::{Authentication, JwtAuthenticator};
pub use geoledger_engine::{Application, Error, MAX_BYTES, Result, Storage};
pub use http::router;
pub use rpc::grpc;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
pub use tokens::Tokens;
use tokio::sync::Semaphore;
fn bad() -> Error {
    Error::new(400, "invalid request")
}
fn text(s: &str, max: usize) -> Result<()> {
    if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
        Err(bad())
    } else {
        Ok(())
    }
}
#[derive(Clone)]
pub struct Service {
    app: Application,
    authentication: Arc<Authentication>,
    capacity: Arc<Semaphore>,
    metrics: Arc<telemetry::Metrics>,
}
impl Service {
    pub fn new(app: Application, authentication: Authentication) -> Self {
        Self {
            app,
            authentication: Arc::new(authentication),
            capacity: Arc::new(Semaphore::new(20)),
            metrics: Arc::new(telemetry::Metrics::default()),
        }
    }
    fn authenticate(&self, headers: &axum::http::HeaderMap) -> Result<String> {
        self.authentication
            .authenticate(headers)
            .ok_or_else(|| Error::new(401, "bearer authentication required"))
    }
    fn permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        self.capacity.clone().try_acquire_owned().map_err(|_| {
            self.metrics.observe(429, 0);
            Error::new(429, "server busy")
        })
    }
    async fn execute(
        &self,
        subject: String,
        op: &str,
        value: Value,
        timeout: Option<Duration>,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Value> {
        let mut app = self.app.clone();
        if let Some(t) = timeout {
            app = app.with_timeout(t.min(Duration::from_secs(30)));
        }
        let op = op.to_owned();
        let started = std::time::Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            app.execute(&subject, &op, value)
        })
        .await
        .map_err(|_| Error::new(500, "operation failed"))?;
        self.metrics.observe(
            result.as_ref().map_or_else(|e| e.status, |_| 200),
            started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        );
        result
    }
}
