//! One server, shared authentication and bounded application execution for HTTP and gRPC.
mod auth;
mod http;
mod limits;
mod resources;
mod rpc;
mod telemetry;
pub mod tls;
mod tokens;
pub use auth::{Authentication, Authenticator, JwtAuthenticator, fetch_jwks};
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
pub use tokens::{Tokens, sha256_hex};
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
    authentication: Arc<Authentication>,
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
    pub fn new(app: Application, authentication: Authentication) -> Self {
        Self::with_authentication(app, Arc::new(authentication))
    }
    /// Share a reloadable authenticator with the process that rotates credentials.
    pub fn with_authentication(app: Application, authentication: Arc<Authentication>) -> Self {
        Self {
            app,
            authentication,
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
    fn admit(&self, call: &mut Call, headers: &axum::http::HeaderMap) -> Result<ExecutionPermit> {
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
        let subject = self.authentication.authenticate(headers).ok_or_else(|| {
            self.metrics.auth_failure();
            Error::new(401, "bearer authentication required")
        })?;
        call.subject = Some(subject.clone());
        if let Some(limiter) = &self.subject_limiter
            && !limiter.allow(&subject)
        {
            self.metrics.rate_limited(false);
            return Err(Error::new(429, "rate limit exceeded"));
        }
        let slot = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::new(429, "server busy"))?;
        let memory = self.requests.reserve(0)?;
        Ok(ExecutionPermit {
            _slot: slot,
            memory,
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
        let subject = call.subject.clone().unwrap_or_default();
        let worker = self.clone();
        let call = call.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _in_flight = worker.metrics.in_flight();
            // Completion belongs to the operation even when its waiter disappears.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return Err(Error::new(504, "operation deadline exceeded"));
                }
                app.with_timeout(remaining).execute(&subject, &op, value)
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
