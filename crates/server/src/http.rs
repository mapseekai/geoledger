use crate::{MAX_REQUEST_BYTES, Service, public_message};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use spatial_version::Command;
use spatial_version_core::Error;

pub fn router(service: Service) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { Json(json!({"ok":true,"version":"0.1.0","api":"v1"})) }),
        )
        .route("/v1/commands", post(execute))
        .route("/v1/status", get(status))
        .route("/v1/log", get(log))
        .route("/v1/branches", get(branches).post(branch))
        .route("/v1/conflicts", get(conflicts))
        .route("/v1/commits", post(commit))
        .route("/v1/merges", post(merge))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(service.clone(), authorize))
        .with_state(service)
}
async fn authorize(State(service): State<Service>, request: Request, next: Next) -> Response {
    if !service.authorized(
        request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":{"code":"unauthenticated","message":"Bearer token required"}})),
        )
            .into_response();
    }
    next.run(request).await
}
pub struct ApiError(Error);
impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::Invalid(_) | Error::Json(_) => StatusCode::BAD_REQUEST,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Dirty | Error::Conflict(_) | Error::Recovery(_) => StatusCode::CONFLICT,
            Error::Busy => StatusCode::LOCKED,
            Error::Unsupported(_) => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(json!({"error":{"code":self.0.code(),"message":public_message(&self.0)}})),
        )
            .into_response()
    }
}
async fn run(service: Service, command: Command) -> Result<Json<Value>, ApiError> {
    Ok(Json(service.execute(command).await?))
}
async fn execute(
    State(service): State<Service>,
    Json(command): Json<Command>,
) -> Result<Json<Value>, ApiError> {
    run(service, command).await
}
#[derive(Deserialize)]
struct Page {
    #[serde(default = "default_limit")]
    limit: usize,
}
fn default_limit() -> usize {
    100
}
async fn status(State(s): State<Service>, Query(p): Query<Page>) -> Result<Json<Value>, ApiError> {
    run(s, Command::Status { limit: p.limit }).await
}
async fn conflicts(
    State(s): State<Service>,
    Query(p): Query<Page>,
) -> Result<Json<Value>, ApiError> {
    run(s, Command::Conflicts { limit: p.limit }).await
}
#[derive(Deserialize)]
struct History {
    #[serde(default = "head")]
    reference: String,
    #[serde(default = "default_limit")]
    limit: usize,
}
fn head() -> String {
    "HEAD".into()
}
async fn log(State(s): State<Service>, Query(p): Query<History>) -> Result<Json<Value>, ApiError> {
    run(
        s,
        Command::Log {
            reference: p.reference,
            limit: p.limit,
        },
    )
    .await
}
async fn branches(State(s): State<Service>) -> Result<Json<Value>, ApiError> {
    run(s, Command::Branches).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewBranch {
    name: String,
    #[serde(default = "head")]
    from: String,
}
async fn branch(
    State(s): State<Service>,
    Json(p): Json<NewBranch>,
) -> Result<Json<Value>, ApiError> {
    run(
        s,
        Command::Branch {
            name: p.name,
            from: p.from,
        },
    )
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewCommit {
    message: String,
    #[serde(default = "unknown")]
    author: String,
}
fn unknown() -> String {
    "unknown".into()
}
async fn commit(
    State(s): State<Service>,
    Json(p): Json<NewCommit>,
) -> Result<Json<Value>, ApiError> {
    run(
        s,
        Command::Commit {
            message: p.message,
            author: p.author,
        },
    )
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewMerge {
    source: String,
    #[serde(default = "unknown")]
    author: String,
    #[serde(default)]
    message: Option<String>,
}
async fn merge(State(s): State<Service>, Json(p): Json<NewMerge>) -> Result<Json<Value>, ApiError> {
    run(
        s,
        Command::Merge {
            source: p.source,
            author: p.author,
            message: p.message,
        },
    )
    .await
}
