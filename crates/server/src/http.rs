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
    let result = handle(&s, &mut call, &op, request).await;
    match s.finish(&call, result) {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn handle(s: &Service, call: &mut Call, op: &str, request: Request) -> Result<Value> {
    let permit = s.admit(call, request.headers())?;
    let body = match tokio::time::timeout(
        s.limits.request_timeout,
        axum::body::to_bytes(request.into_body(), MAX_BYTES),
    )
    .await
    {
        Err(_) => return Err(Error::new(408, "request body timeout")),
        Ok(Err(_)) => return Err(Error::new(413, "request body too large")),
        Ok(Ok(b)) => b,
    };
    let input = geoledger_engine::parse_json(&body)?;
    if op == "info" {
        return Ok(
            json!({"version":env!("CARGO_PKG_VERSION"),"backend":s.app.backend(),"format_version":geoledger_engine::FORMAT_VERSION}),
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
    if s.authentication.authenticate(&headers).is_none() {
        s.metrics.auth_failure();
        return error(Error::new(401, "bearer authentication required"));
    }
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        s.metrics.render(&s.gauges()),
    )
        .into_response()
}
