use crate::{MAX_REQUEST_BYTES, Service, public_message};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Query, Request, State},
    extract::{
        FromRequest, FromRequestParts,
        rejection::{JsonRejection, QueryRejection},
    },
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use geoledger::Command;
use geoledger_core::Error;
use serde::{Deserialize, de::DeserializeOwned};
use std::sync::atomic::{AtomicU64, Ordering};

tokio::task_local! { static REQUEST_ID: String; }
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
use serde_json::json;

pub fn router(service: Service) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async {
                Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"api":"v1"}))
            }),
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
        .layer(middleware::from_fn(correlate))
        .with_state(service)
}
async fn authorize(State(service): State<Service>, request: Request, next: Next) -> Response {
    if !service.authorized(
        request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok()),
    ) {
        return envelope(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Bearer token required",
        );
    }
    let Ok(_permit) = service.ingress.try_acquire_owned() else {
        return ApiError::Application(Error::Busy).into_response();
    };
    next.run(request).await
}
async fn correlate(request: Request, next: Next) -> Response {
    // Generate locally: never reflect attacker-controlled IDs into logs/headers.
    let id = format!(
        "local-{}-{}",
        std::process::id(),
        NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
    );
    REQUEST_ID
        .scope(id.clone(), async move {
            let mut response = next.run(request).await;
            if let Ok(value) = id.parse() {
                response.headers_mut().insert("x-request-id", value);
            }
            response
        })
        .await
}
pub(crate) fn request_id() -> Option<String> {
    REQUEST_ID.try_with(Clone::clone).ok()
}
fn envelope(status: StatusCode, code: &str, message: &str) -> Response {
    let request_id = REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| "unscoped".into());
    (
        status,
        Json(json!({"error":{"code":code,"message":message,"request_id":request_id}})),
    )
        .into_response()
}
pub enum ApiError {
    Application(Error),
    Extraction(StatusCode),
}
impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self::Application(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let error = match self {
            Self::Application(error) => error,
            Self::Extraction(status) => {
                let (code, message) = match status {
                    StatusCode::REQUEST_TIMEOUT => ("deadline_exceeded", "request body timeout"),
                    StatusCode::PAYLOAD_TOO_LARGE => ("payload_too_large", "request exceeds 4 MiB"),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE => (
                        "unsupported_media_type",
                        "application/json content type required",
                    ),
                    StatusCode::UNPROCESSABLE_ENTITY => {
                        ("invalid_argument", "JSON does not match the request schema")
                    }
                    _ => ("invalid_argument", "malformed request"),
                };
                return envelope(status, code, message);
            }
        };
        let status = match &error {
            Error::Invalid(_) | Error::Json(_) => StatusCode::BAD_REQUEST,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Dirty | Error::Conflict(_) | Error::Recovery(_) => StatusCode::CONFLICT,
            Error::Busy => StatusCode::LOCKED,
            Error::Unsupported(_) => StatusCode::UNPROCESSABLE_ENTITY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        envelope(status, error.code(), &public_message(&error))
    }
}
struct ApiJson<T>(T);
impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;
    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Json::<T>::from_request(request, state),
        )
        .await
        .map_err(|_| ApiError::Extraction(StatusCode::REQUEST_TIMEOUT))?
        .map(|Json(value)| Self(value))
        .map_err(|error: JsonRejection| ApiError::Extraction(error.status()))
    }
}
struct ApiQuery<T>(T);
impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|error: QueryRejection| ApiError::Extraction(error.status()))
    }
}
async fn run(service: Service, command: Command) -> Result<Response, ApiError> {
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        service.execute_json(command).await?,
    )
        .into_response())
}
async fn execute(
    State(service): State<Service>,
    ApiJson(command): ApiJson<Command>,
) -> Result<Response, ApiError> {
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
async fn status(
    State(s): State<Service>,
    ApiQuery(p): ApiQuery<Page>,
) -> Result<Response, ApiError> {
    run(s, Command::Status { limit: p.limit }).await
}
async fn conflicts(
    State(s): State<Service>,
    ApiQuery(p): ApiQuery<Page>,
) -> Result<Response, ApiError> {
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
async fn log(
    State(s): State<Service>,
    ApiQuery(p): ApiQuery<History>,
) -> Result<Response, ApiError> {
    run(
        s,
        Command::Log {
            reference: p.reference,
            limit: p.limit,
        },
    )
    .await
}
async fn branches(State(s): State<Service>) -> Result<Response, ApiError> {
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
    ApiJson(p): ApiJson<NewBranch>,
) -> Result<Response, ApiError> {
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
    #[serde(default = "geoledger::default_author")]
    author: String,
}
async fn commit(
    State(s): State<Service>,
    ApiJson(p): ApiJson<NewCommit>,
) -> Result<Response, ApiError> {
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
    #[serde(default = "geoledger::default_author")]
    author: String,
    #[serde(default)]
    message: Option<String>,
}
async fn merge(
    State(s): State<Service>,
    ApiJson(p): ApiJson<NewMerge>,
) -> Result<Response, ApiError> {
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

#[cfg(test)]
mod tests {
    #[test]
    fn commit_and_merge_authors_default_to_mapseekai() -> Result<(), serde_json::Error> {
        let commit: super::NewCommit = serde_json::from_str(r#"{"message":"save"}"#)?;
        let merge: super::NewMerge = serde_json::from_str(r#"{"source":"draft"}"#)?;
        assert_eq!(commit.author, "mapseekai");
        assert_eq!(merge.author, "mapseekai");
        let explicit: super::NewCommit =
            serde_json::from_str(r#"{"message":"save","author":"custom-author"}"#)?;
        assert_eq!(explicit.author, "custom-author");
        Ok(())
    }
}
