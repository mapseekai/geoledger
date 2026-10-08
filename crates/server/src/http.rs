use crate::*;
use axum::{
    Router,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
pub fn router(service: Service) -> Router {
    Router::new()
        .route("/", get(console))
        .route("/logo.png", get(logo))
        .route(
            "/health",
            get(|| async { axum::Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION")})) }),
        )
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .route("/api/v1/{operation}", post(call))
        .layer(middleware::from_fn(headers))
        .with_state(service)
}
async fn headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let id = uuid::Uuid::new_v4().to_string();
    if !response.headers().contains_key("x-request-id")
        && let Ok(id) = id.parse()
    {
        response.headers_mut().insert("x-request-id", id);
    }
    response.headers_mut().insert(
        "x-content-type-options",
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response
}
async fn console() -> Response {
    (
 [("cache-control","no-store"),("content-security-policy","default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")],
 axum::response::Html(include_str!("../web/index.html"))).into_response()
}
async fn logo() -> impl IntoResponse {
    (
        [
            ("content-type", "image/png"),
            ("cache-control", "public, max-age=3600"),
        ],
        include_bytes!("../web/logo.png").as_slice(),
    )
}
fn error(e: Error) -> Response {
    let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let id = e.body["error"]["request_id"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    (status, [("x-request-id", id)], axum::Json(e.body)).into_response()
}
async fn call(State(s): State<Service>, Path(op): Path<String>, request: Request) -> Response {
    let subject = match s.authenticate(request.headers()) {
        Ok(s) => s,
        Err(e) => return error(e),
    };
    let permit = match s.permit() {
        Ok(p) => p,
        Err(e) => return error(e),
    };
    let body = match tokio::time::timeout(
        Duration::from_secs(30),
        axum::body::to_bytes(request.into_body(), MAX_BYTES),
    )
    .await
    {
        Err(_) => return error(Error::new(408, "request body timeout")),
        Ok(Err(_)) => return error(Error::new(413, "request body too large")),
        Ok(Ok(b)) => b,
    };
    let input = match geoledger_engine::parse_json(&body) {
        Ok(v) => v,
        Err(e) => return error(e),
    };
    if op == "info" {
        return axum::Json(json!({"version":env!("CARGO_PKG_VERSION"),"backend":s.app.backend(),"format_version":geoledger_engine::FORMAT_VERSION})).into_response();
    }
    match s.execute(subject, &op, input, None, permit).await {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => error(e),
    }
}
async fn ready(State(s): State<Service>) -> Response {
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
        _ => error(Error::new(503, "storage not ready")),
    }
}
async fn metrics(State(s): State<Service>, headers: HeaderMap) -> Response {
    if let Err(e) = s.authenticate(&headers) {
        return error(e);
    }
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        s.metrics.render(s.capacity.available_permits()),
    )
        .into_response()
}
