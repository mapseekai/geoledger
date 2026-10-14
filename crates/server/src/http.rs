use crate::*;
use axum::{
    Router,
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use sha2::{Digest, Sha256};
/// Transport request ID, accepted from `x-request-id` or minted per request.
#[derive(Clone)]
struct RequestId(String);
pub fn router(service: Service) -> Router {
    Router::new()
        .route("/", get(discovery))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/api/v1/{operation}", post(call))
        .route("/auth/v1/{operation}", post(auth_call))
        .layer(middleware::from_fn(headers))
        .with_state(service)
}
/// Unauthenticated liveness/readiness only, for a separate plaintext probe listener.
pub fn health_router(service: Service) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .layer(middleware::from_fn(headers))
        .with_state(service)
}
async fn headers(mut req: Request, next: Next) -> Response {
    let id = limits::request_id(
        req.headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
    );
    req.extensions_mut().insert(RequestId(id.clone()));
    let mut response = next.run(req).await;
    if let Ok(id) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", id);
    }
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}
async fn health() -> impl IntoResponse {
    axum::Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION")}))
}
async fn discovery() -> impl IntoResponse {
    axum::Json(json!({
        "service": "geoledger-server",
        "version": env!("CARGO_PKG_VERSION"),
        "console": "Deploy the separate GeoLedger web application",
        "health": "/health", "ready": "/ready"
    }))
}
fn error(e: Error) -> Response {
    let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, axum::Json(e.body)).into_response();
    if status == StatusCode::TOO_MANY_REQUESTS {
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static("1"));
    }
    response
}
async fn call(State(s): State<Service>, Path(op): Path<String>, request: Request) -> Response {
    let id = request
        .extensions()
        .get::<RequestId>()
        .map_or_else(|| limits::request_id(None), |r| r.0.clone());
    let peer = request
        .extensions()
        .get::<ConnectInfo<tls::PeerAddr>>()
        .map(|c| c.0.0.ip());
    let mut call = Call::new("http", &op, id, peer);
    let result = async {
        let permit = s.admit(&mut call, request.headers()).await?;
        let lease = s.responses.reserve(0)?;
        call.response_lease = Some(lease.clone());
        let value = handle(&s, &mut call, &op, request, permit).await?;
        let bytes =
            serde_json::to_vec(&value).map_err(|_| Error::new(500, "response encoding failed"))?;
        if bytes.len() > s.limits.max_response_bytes {
            return Err(Error::new(
                413,
                "response exceeds byte budget; reduce page size",
            ));
        }
        lease.grow(bytes.len())?;
        lease.shrink(bytes.len());
        Ok((bytes, lease))
    }
    .await;
    finish_response(&s, &call, result)
}
fn finish_response(
    s: &Service,
    call: &Call,
    result: Result<(Vec<u8>, resources::ByteLease)>,
) -> Response {
    match s.finish(call, result) {
        Ok((bytes, lease)) => {
            let mut response = Response::new(axum::body::Body::new(resources::LeasedBody::new(
                axum::body::Body::from(bytes),
                lease,
            )));
            response
                .headers_mut()
                .insert("content-type", HeaderValue::from_static("application/json"));
            response
        }
        Err(e) => {
            use http_body::Body as _;
            let mut response = error(e);
            if let Some(lease) = &call.response_lease {
                let bytes = response
                    .body()
                    .size_hint()
                    .exact()
                    .unwrap_or(0)
                    .min(usize::MAX as u64) as usize;
                if bytes > s.limits.max_response_bytes {
                    response = error(
                        Error::new(
                            413,
                            "error response exceeds byte budget; use paginated details",
                        )
                        .with_request_id(&call.request_id),
                    );
                    lease.shrink(0);
                } else if let Err(e) = lease.grow(bytes) {
                    response = error(e.with_request_id(&call.request_id));
                    lease.shrink(0);
                } else {
                    lease.shrink(bytes);
                }
                return response.map(|body| {
                    axum::body::Body::new(resources::LeasedBody::new(body, lease.clone()))
                });
            }
            response
        }
    }
}
async fn handle(
    s: &Service,
    call: &mut Call,
    op: &str,
    request: Request,
    permit: ExecutionPermit,
) -> Result<Value> {
    let body = match tokio::time::timeout(
        s.limits
            .request_timeout
            .saturating_sub(call.started.elapsed()),
        axum::body::to_bytes(
            axum::body::Body::new(resources::BudgetedBody::new(
                request.into_body(),
                permit.memory.clone(),
                usize::MAX,
            )),
            s.limits.max_request_bytes,
        ),
    )
    .await
    {
        Err(_) => return Err(Error::new(408, "request body timeout")),
        Ok(Err(e)) => {
            use std::error::Error as _;
            if e.source()
                .is_some_and(|e| e.is::<http_body_util::LengthLimitError>())
            {
                return Err(Error::new(413, "request exceeds byte budget"));
            }
            let mut source = e.source();
            while let Some(error) = source {
                if error
                    .downcast_ref::<tonic::Status>()
                    .is_some_and(|e| e.code() == tonic::Code::ResourceExhausted)
                {
                    return Err(Error::new(429, "receive memory budget exhausted"));
                }
                source = error.source();
            }
            return Err(Error::new(400, "failed to read request body"));
        }
        Ok(Ok(b)) => b,
    };
    let input = geoledger_engine::parse_json(&body)?;
    if op == "info" {
        return Ok(
            json!({"version":env!("CARGO_PKG_VERSION"),"backend":s.app.backend(),"format_version":geoledger_engine::FORMAT_VERSION,"platform_admin":call.identity.as_ref().is_some_and(|v|v.user.is_admin),"max_request_bytes":s.limits.max_request_bytes,"max_feature_bytes":0}),
        );
    }
    s.execute(call, input, None, permit).await
}
async fn ready(State(s): State<Service>) -> Response {
    if s.is_draining() {
        return error(Error::new(503, "server draining"));
    }
    let permit = match s.capacity.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return error(Error::new(503, "server busy")),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        s.app.with_timeout(Duration::from_secs(2)).check_schema()
    })
    .await;
    match result {
        Ok(Ok(())) => axum::Json(json!({"ok":true})).into_response(),
        Ok(Err(e)) => {
            tracing::warn!(
                status = e.status,
                diagnostic = e.diagnostic().as_deref().unwrap_or("-"),
                "readiness check failed"
            );
            error(Error::new(503, "storage not ready"))
        }
        Err(_) => error(Error::new(503, "storage not ready")),
    }
}
async fn metrics(State(s): State<Service>, headers: HeaderMap) -> Response {
    let credential = match auth::credential(&headers, true) {
        Ok(value) => value,
        Err(e) => {
            s.metrics.auth_failure();
            return error(e);
        }
    };
    // Scraping remains available while business slots and rate buckets are exhausted.
    let permit = match s.password_capacity.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return error(Error::new(429, "authentication busy")),
    };
    let app = s.app.clone();
    let authenticated = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        match credential {
            auth::Credential::ApiKey(key) => app.authenticate_api_key(&key),
            auth::Credential::Session(session) => app.authenticate_session(&session),
        }
    })
    .await;
    match authenticated {
        Ok(Ok(identity)) if !identity.user.must_change_password => {}
        Ok(Ok(_)) => return error(Error::new(403, "password change required")),
        Ok(Err(e)) => {
            s.metrics.auth_failure();
            return error(e);
        }
        Err(_) => return error(Error::new(500, "authentication unavailable")),
    }
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        s.metrics.render(&s.gauges()),
    )
        .into_response()
}

/// Console-only transport; the public SDK exposes no session or password login.
async fn auth_call(State(s): State<Service>, Path(op): Path<String>, request: Request) -> Response {
    let id = request
        .extensions()
        .get::<RequestId>()
        .map_or_else(|| limits::request_id(None), |r| r.0.clone());
    let peer = request
        .extensions()
        .get::<ConnectInfo<tls::PeerAddr>>()
        .map(|c| c.0.0.ip());
    let mut call = Call::new("http", &format!("auth:{op}"), id, peer);
    let result = async {
        let anonymous = matches!(op.as_str(), "login_password" | "login_api_key");
        let sensitive = !matches!(
            op.as_str(),
            "me" | "logout" | "list_users" | "list_api_keys"
        );
        let peer = limits::client_ip(
            peer,
            request
                .headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok()),
            s.limits.trust_forwarded_for,
        );
        if anonymous
            && !s.login_peer_limiter.allow(&format!(
                "ip:{}",
                peer.map_or_else(|| "local".into(), |ip| ip.to_string())
            ))
        {
            return Err(Error::new(429, "authentication rate limit exceeded"));
        }
        let permit = if anonymous {
            if request.headers().contains_key("authorization")
                || request.headers().contains_key("x-geoledger-session")
            {
                return Err(Error::new(401, "authentication failed"));
            }
            ExecutionPermit {
                _slot: s
                    .capacity
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| Error::new(429, "server busy"))?,
                memory: s.requests.reserve(0)?,
            }
        } else {
            s.admit(&mut call, request.headers()).await?
        };
        let lease = s.responses.reserve(0)?;
        call.response_lease = Some(lease.clone());
        if sensitive
            && let Some(subject) = &call.subject
            && !s.auth_limiter.allow(&format!("user:{subject}"))
        {
            return Err(Error::new(429, "authentication rate limit exceeded"));
        }
        let body = tokio::time::timeout(
            s.limits
                .request_timeout
                .saturating_sub(call.started.elapsed()),
            axum::body::to_bytes(
                axum::body::Body::new(resources::BudgetedBody::new(
                    request.into_body(),
                    permit.memory.clone(),
                    usize::MAX,
                )),
                s.limits.max_request_bytes.min(16 * 1024),
            ),
        )
        .await
        .map_err(|_| Error::new(408, "request body timeout"))?
        .map_err(|_| Error::new(413, "request exceeds byte budget"))?;
        let input = geoledger_engine::parse_json(&body)?;
        if op == "login_password" {
            let username = input
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            // Match engine normalization; retain only a bounded digest, even for invalid names.
            let digest: String = Sha256::digest(username.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let bucket = format!("username:{digest}");
            if !s.auth_limiter.allow(&bucket) {
                return Err(Error::new(429, "authentication rate limit exceeded"));
            }
        }
        let password_permit = if sensitive && op != "login_api_key" {
            Some(
                s.password_capacity
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| Error::new(429, "authentication busy"))?,
            )
        } else {
            None
        };
        let app = s
            .app
            .clone()
            .with_response_limit(s.limits.max_response_bytes)
            .with_response_reservation(lease.reservation());
        let deadline = call.started + s.limits.request_timeout;
        let identity = call.identity.clone();
        let value = tokio::task::spawn_blocking(move || {
            let (_permit, _password_permit) = (permit, password_permit);
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::new(504, "operation deadline exceeded"));
            }
            app.with_timeout(remaining)
                .auth_operation(identity.as_ref(), &op, input)
        })
        .await
        .map_err(|_| Error::new(500, "authentication unavailable"))??;
        let bytes =
            serde_json::to_vec(&value).map_err(|_| Error::new(500, "response encoding failed"))?;
        lease.grow(bytes.len())?;
        lease.shrink(bytes.len());
        Ok((bytes, lease))
    }
    .await;
    finish_response(&s, &call, result)
}
