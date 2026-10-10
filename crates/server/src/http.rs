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
    let result = async {
        let permit = s.admit(&mut call, request.headers())?;
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
    match s.finish(&call, result) {
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
            json!({"version":env!("CARGO_PKG_VERSION"),"backend":s.app.backend(),"format_version":geoledger_engine::FORMAT_VERSION,"max_request_bytes":s.limits.max_request_bytes,"max_feature_bytes":0}),
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
