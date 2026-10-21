//! One server, shared authentication and bounded application execution for HTTP and gRPC.
mod auth;
mod http;
mod limits;
mod resources;
mod rpc;
mod telemetry;
pub mod tls;

pub use geoledger_engine::{Application, Error, Policy, Result, Storage, StorageOptions};
pub use http::{health_router, router};
pub use limits::{Limits, Rate};
pub use rpc::{Grpc, grpc};
use serde_json::Value;
use std::{
    net::IpAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::sync::Semaphore;
fn bad() -> Error {
    Error::new(400, "invalid request")
}
/// Per-call context shared by logging, metrics and error correlation.
#[derive(Clone)]
pub(crate) struct Call {
    finished: Arc<AtomicBool>,
    response_lease: Option<resources::ByteLease>,
    pub protocol: &'static str,
    pub operation: String,
    pub request_id: String,
    pub peer: Option<IpAddr>,
    pub started: std::time::Instant,
    pub subject: Option<String>,
    pub identity: Option<geoledger_engine::AuthenticatedIdentity>,
}
impl Call {
    pub fn new(
        protocol: &'static str,
        operation: &str,
        request_id: String,
        peer: Option<IpAddr>,
    ) -> Self {
        Self {
            finished: Arc::new(AtomicBool::new(false)),
            response_lease: None,
            protocol,
            operation: operation.to_owned(),
            request_id,
            peer,
            started: std::time::Instant::now(),
            subject: None,
            identity: None,
        }
    }
}
pub(crate) struct ExecutionPermit {
    _slot: tokio::sync::OwnedSemaphorePermit,
    memory: resources::ByteLease,
}
#[derive(Clone)]
pub struct Service {
    app: Application,
    password_capacity: Arc<Semaphore>,
    auth_limiter: Arc<limits::Limiter>,
    login_peer_limiter: Arc<limits::Limiter>,
    capacity: Arc<Semaphore>,
    requests: resources::ByteBudget,
    responses: resources::ByteBudget,
    metrics: Arc<telemetry::Metrics>,
    limits: Arc<Limits>,
    subject_limiter: Option<Arc<limits::Limiter>>,
    ip_limiter: Option<Arc<limits::Limiter>>,
    draining: Arc<AtomicBool>,
}
impl Service {
    pub fn new(app: Application) -> Self {
        Self {
            app,
            password_capacity: Arc::new(Semaphore::new(4)),
            auth_limiter: Arc::new(limits::Limiter::new(Rate {
                per_second: 0.2,
                burst: 10.0,
            })),
            login_peer_limiter: Arc::new(limits::Limiter::new(Rate {
                per_second: 10.0,
                burst: 100.0,
            })),
            requests: resources::ByteBudget::new(Limits::default().request_memory_bytes),
            responses: resources::ByteBudget::new(Limits::default().response_memory_bytes),
            capacity: Arc::new(Semaphore::new(Limits::default().max_concurrency)),
            metrics: Arc::new(telemetry::Metrics::default()),
            limits: Arc::new(Limits::default()),
            subject_limiter: None,
            ip_limiter: None,
            draining: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Apply concurrency, timeout and rate limits. Call before cloning the service.
    pub fn with_limits(mut self, limits: Limits) -> Self {
        let limits = Limits {
            max_concurrency: limits.max_concurrency.max(1),
            request_timeout: limits.request_timeout.max(Duration::from_secs(1)),
            max_request_bytes: limits.max_request_bytes.clamp(1, u32::MAX as usize),
            max_response_bytes: limits.max_response_bytes.clamp(1, u32::MAX as usize),
            ..limits
        };
        let limits = Limits {
            request_memory_bytes: limits
                .request_memory_bytes
                .max(limits.max_request_bytes)
                .min(u32::MAX as usize),
            response_memory_bytes: limits
                .response_memory_bytes
                .max(limits.max_response_bytes)
                .min(u32::MAX as usize),
            ..limits
        };
        self.requests = resources::ByteBudget::new(limits.request_memory_bytes);
        self.responses = resources::ByteBudget::new(limits.response_memory_bytes);
        self.capacity = Arc::new(Semaphore::new(limits.max_concurrency));
        self.subject_limiter = limits
            .per_subject
            .map(|r| Arc::new(limits::Limiter::new(r)));
        self.ip_limiter = limits.per_ip.map(|r| Arc::new(limits::Limiter::new(r)));
        self.limits = Arc::new(limits);
        self
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    /// Stop reporting ready so load balancers drain traffic before shutdown.
    pub fn start_draining(&self) {
        self.draining.store(true, Ordering::SeqCst);
    }
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }
    /// Rate-limit, authenticate and reserve an execution slot for one call.
    async fn admit(
        &self,
        call: &mut Call,
        headers: &axum::http::HeaderMap,
    ) -> Result<ExecutionPermit> {
        let ip = limits::client_ip(
            call.peer,
            headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()),
            self.limits.trust_forwarded_for,
        );
        call.peer = ip;
        if let (Some(limiter), Some(ip)) = (&self.ip_limiter, ip)
            && !limiter.allow(&ip.to_string())
        {
            self.metrics.rate_limited(true);
            return Err(Error::new(429, "rate limit exceeded"));
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(429, "server busy"))?;
        let credential = auth::credential(headers, call.protocol == "http")
            .inspect_err(|_| self.metrics.auth_failure())?;
        let app = self.app.clone();
        let deadline = call
            .started
            .checked_add(self.limits.request_timeout)
            .ok_or_else(|| Error::new(400, "invalid operation deadline"))?;
        let (identity, permit) = tokio::task::spawn_blocking(move || {
            // The blocking queue consumes the same request budget as database work.
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let identity = if remaining.is_zero() {
                Err(Error::new(504, "operation deadline exceeded"))
            } else {
                let app = app.with_timeout(remaining);
                match credential {
                    auth::Credential::ApiKey(key) => app.authenticate_api_key(&key),
                    auth::Credential::Session(session) => app.authenticate_session(&session),
                }
            };
            (identity, permit)
        })
        .await
        .map_err(|_| Error::new(500, "authentication unavailable"))?;
        let identity = identity.inspect_err(|_| self.metrics.auth_failure())?;
        if identity.user.must_change_password && !call.operation.starts_with("auth:") {
            return Err(Error::new(403, "password change required"));
        }
        let subject = identity.user.id.clone();
        call.subject = Some(subject.clone());
        call.identity = Some(identity);
        if let Some(limiter) = &self.subject_limiter
            && !limiter.allow(&subject)
        {
            self.metrics.rate_limited(false);
            return Err(Error::new(429, "rate limit exceeded"));
        }
        Ok(ExecutionPermit {
            _slot: permit,
            memory: self.requests.reserve(0)?,
        })
    }
    /// Finish a call: correlate the error with the transport request ID, log
    /// server-side failures with a safe diagnostic, emit metrics and an access log.
    fn finish<T>(&self, call: &Call, result: Result<T>) -> Result<T> {
        let result = result.map_err(|e| e.with_request_id(&call.request_id));
        if call.finished.swap(true, Ordering::AcqRel) {
            return result;
        }
        let status = result.as_ref().map_or_else(|e| e.status, |_| 200);
        let micros = call.started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        self.metrics
            .observe(call.protocol, &call.operation, status, micros);
        let operation = telemetry::operation_label(&call.operation);
        if let Err(e) = &result
            && e.status >= 500
        {
            tracing::error!(
                request_id = %call.request_id,
                protocol = call.protocol,
                operation,
                status,
                subject = call.subject.as_deref().unwrap_or("-"),
                diagnostic = e.diagnostic().as_deref().unwrap_or("-"),
                "operation failed"
            );
        }
        tracing::info!(
            target: "geoledger_server::access",
            request_id = %call.request_id,
            protocol = call.protocol,
            operation,
            status,
            duration_ms = micros as f64 / 1000.0,
            subject = call.subject.as_deref().unwrap_or("-"),
            peer = %call.peer.map_or_else(|| "-".to_owned(), |ip| ip.to_string()),
            "request"
        );
        result
    }
    async fn execute(
        &self,
        call: &Call,
        value: Value,
        timeout: Option<Duration>,
        permit: ExecutionPermit,
    ) -> Result<Value> {
        let max = self.limits.request_timeout;
        let mut app = self
            .app
            .clone()
            .with_response_limit(self.limits.max_response_bytes);
        if let Some(lease) = &call.response_lease {
            app = app.with_response_reservation(lease.reservation());
        }

        let deadline = call
            .started
            .checked_add(timeout.map_or(max, |t| t.min(max)))
            .ok_or_else(|| Error::new(400, "invalid operation deadline"))?;
        let op = call.operation.clone();
        let identity = call
            .identity
            .clone()
            .ok_or_else(|| Error::new(401, "authentication failed"))?;
        let password_permit = if identity.session_id.is_some()
            && matches!(op.as_str(), "set_member" | "remove_member")
        {
            if !self
                .auth_limiter
                .allow(&format!("user:{}", identity.user.id))
            {
                return Err(Error::new(429, "authentication rate limit exceeded"));
            }
            Some(
                self.password_capacity
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| Error::new(429, "authentication busy"))?,
            )
        } else {
            None
        };
        let worker = self.clone();
        let call = call.clone();
        tokio::task::spawn_blocking(move || {
            let (_permit, _password_permit) = (permit, password_permit);
            let _in_flight = worker.metrics.in_flight();
            // Completion belongs to the operation even when its waiter disappears.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return Err(Error::new(504, "operation deadline exceeded"));
                }
                app.with_timeout(remaining)
                    .execute_authenticated(&identity, &op, value)
            }))
            .unwrap_or_else(|_| Err(Error::new(500, "operation failed")));
            worker.finish(&call, result)
        })
        .await
        .map_err(|_| Error::new(500, "operation failed"))?
    }
    fn gauges(&self) -> telemetry::Gauges {
        telemetry::Gauges {
            slots_available: self.capacity.available_permits(),
            slots_capacity: self.limits.max_concurrency,
            draining: self.is_draining(),
            pool: self.app.pool_stats(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geoledger_engine::repository::{RepositoryTransaction, StorageBackend};
    use std::{future::Future, sync::atomic::AtomicUsize, task::Poll};

    struct CountingStorage(AtomicUsize);
    impl geoledger_engine::repository::StorageMaintenance for CountingStorage {}
    impl StorageBackend for CountingStorage {
        fn name(&self) -> &'static str {
            "test"
        }
        fn initialize(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        fn health(&self, _: Duration) -> Result<()> {
            Ok(())
        }
        fn begin(&self, _: bool, _: Duration) -> Result<Box<dyn RepositoryTransaction>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::new(401, "test authentication failed"))
        }
    }

    #[test]
    fn queued_authentication_expires_before_storage_and_releases_permit()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .build()?;
        runtime.block_on(async {
            for (protocol, header, credential) in [
                (
                    "http",
                    "authorization",
                    format!("Bearer glk_{}", "a".repeat(64)),
                ),
                (
                    "http",
                    "x-geoledger-session",
                    format!("gls_{}", "a".repeat(64)),
                ),
                (
                    "grpc",
                    "authorization",
                    format!("Bearer glk_{}", "a".repeat(64)),
                ),
            ] {
                let storage = Arc::new(CountingStorage(AtomicUsize::new(0)));
                let service =
                    Service::new(Application::with_backend(storage.clone())).with_limits(Limits {
                        max_concurrency: 1,
                        request_timeout: Duration::from_secs(1),
                        ..Limits::default()
                    });
                let (entered, started) = tokio::sync::oneshot::channel();
                let (release, wait) = std::sync::mpsc::channel();
                let worker = tokio::task::spawn_blocking(move || {
                    let _ = entered.send(());
                    // A bounded wait also releases the worker if the test fails early.
                    wait.recv_timeout(Duration::from_secs(5))
                });
                started.await?;
                let mut headers = axum::http::HeaderMap::new();
                headers.insert(header, credential.parse()?);
                let mut call =
                    Call::new(protocol, "list_projects", "queued-auth-test".into(), None);
                let admission = service.admit(&mut call, &headers);
                tokio::pin!(admission);
                // Poll once to enqueue authentication behind the occupied blocking worker.
                let queued = std::future::poll_fn(|cx| {
                    Poll::Ready(admission.as_mut().poll(cx).is_pending())
                })
                .await;
                let held = service.capacity.available_permits();
                tokio::time::sleep(Duration::from_millis(1100)).await;
                release.send(())?;
                worker.await??;
                assert!(queued);
                assert_eq!(held, 0);
                let error = admission.await.err().ok_or("expected expired admission")?;
                assert_eq!(error.status, 504);
                assert_eq!(
                    storage.0.load(Ordering::SeqCst),
                    0,
                    "expired admission reached storage"
                );
                assert_eq!(service.capacity.available_permits(), 1);
            }
            Ok(())
        })
    }
}
