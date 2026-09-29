use crate::{CenterApplication, Error, MAX_BYTES, Result, bad, text};
use axum::{
    Router,
    extract::{Path, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc};
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;

/// Operator-provided secrets are neither serializable nor Debug-printable.
pub struct Tokens(Vec<Token>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    token: String,
    subject: String,
}
impl Tokens {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(bad());
        }
        let entries: Vec<Token> =
            serde_json::from_slice(bytes).map_err(|_| Error::new(400, "invalid token file"))?;
        if entries.is_empty() || entries.len() > 1024 {
            return Err(bad());
        }
        let mut tokens = BTreeSet::new();
        let mut subjects = BTreeSet::new();
        for e in &entries {
            text(&e.subject, 128)?;
            if e.token.len() < 43
                || e.token.len() > 256
                || !e.token.bytes().all(|b| b.is_ascii_graphic())
                || !tokens.insert(&e.token)
                || !subjects.insert(&e.subject)
            {
                return Err(Error::new(
                    400,
                    "tokens must be unique, 43-256 ASCII characters, with unique subjects",
                ));
            }
        }
        Ok(Self(entries))
    }
    fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        if headers.get_all("authorization").iter().count() != 1 {
            return None;
        }
        let token = headers
            .get("authorization")?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?;
        let mut subject = None;
        for e in &self.0 {
            if bool::from(e.token.as_bytes().ct_eq(token.as_bytes())) {
                subject = Some(e.subject.clone());
            }
        }
        subject
    }
}
#[derive(Clone)]
struct Service {
    app: CenterApplication,
    tokens: Arc<Tokens>,
    capacity: Arc<Semaphore>,
}
pub fn router(app: CenterApplication, tokens: Tokens) -> Router {
    Router::new()
        .route("/", get(console))
        .route(
            "/health",
            get(|| async {
                axum::Json(json!({"ok":true,"version":env!("CARGO_PKG_VERSION"),"mode":"center"}))
            }),
        )
        .route("/api/center/{operation}", post(call))
        .with_state(Service {
            app,
            tokens: Arc::new(tokens),
            capacity: Arc::new(Semaphore::new(16)),
        })
}
async fn console() -> Response {
    (
        [("cache-control", "no-store"),
         ("x-content-type-options", "nosniff"),
         ("content-security-policy", "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")],
        axum::response::Html(include_str!("../web/index.html")),
    ).into_response()
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status;
        let encoded = match crate::codec::encode(&self.body) {
            Ok(encoded) => encoded,
            Err(error) => return error.into_response(),
        };
        (
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            [("content-type", "application/json")],
            encoded,
        )
            .into_response()
    }
}
async fn call(
    State(s): State<Service>,
    Path(operation): Path<String>,
    request: Request,
) -> Result<Response> {
    let subject = s
        .tokens
        .authenticate(request.headers())
        .ok_or_else(|| Error::new(401, "bearer authentication required"))?;
    let permit = s
        .capacity
        .try_acquire_owned()
        .map_err(|_| Error::new(429, "server busy"))?;
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        axum::body::to_bytes(request.into_body(), MAX_BYTES),
    )
    .await
    .map_err(|_| Error::new(408, "request body timeout"))?
    .map_err(|_| Error::new(413, "request body too large or unreadable"))?;
    let input = crate::codec::parse(&body)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        match s.app.execute_encoded(&subject, &operation, input) {
            Ok((_, encoded)) => ([("content-type", "application/json")], encoded).into_response(),
            Err(error) => error.into_response(),
        }
    })
    .await
    .map_err(|_| Error::new(500, "request failed"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[tokio::test]
    async fn browser_console_has_isolation_headers_and_needs_no_database()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        use http_body_util::BodyExt;
        let tokens = Tokens::from_json(
            json!([{"token":"a".repeat(43),"subject":"alice"}])
                .to_string()
                .as_bytes(),
        )?;
        let app = router(CenterApplication::new("invalid dsn".into()), tokens);
        let response = app
            .clone()
            .oneshot(Request::get("/").body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            response.headers()["content-security-policy"]
                .to_str()?
                .contains("connect-src 'self'")
        );
        let body = response.into_body().collect().await?.to_bytes();
        let html = String::from_utf8(body.to_vec())?;
        assert!(html.contains("GeoLedger Center"));
        assert!(!html.contains("localStorage"));
        assert!(!html.contains("sessionStorage"));
        let health = app
            .oneshot(Request::get("/health").body(Body::empty())?)
            .await?;
        assert_eq!(health.status(), StatusCode::OK);
        Ok(())
    }
    #[test]
    fn token_validation() {
        assert!(Tokens::from_json(br#"[{"token":"short","subject":"a"}]"#).is_err());
        let token = "a".repeat(43);
        assert!(
            Tokens::from_json(
                json!([{"token":token,"subject":"a"},{"token":token,"subject":"b"}])
                    .to_string()
                    .as_bytes()
            )
            .is_err()
        );
        assert!(Tokens::from_json(json!([{"token":"a".repeat(43),"subject":"a"},{"token":"b".repeat(43),"subject":"a"}]).to_string().as_bytes()).is_err());
    }
    #[tokio::test]
    async fn authentication_precedes_database_and_ignores_identity_headers()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let tokens = Tokens::from_json(
            json!([{"token":"a".repeat(43),"subject":"alice"}])
                .to_string()
                .as_bytes(),
        )?;
        let app = router(CenterApplication::new("invalid dsn".into()), tokens);
        for authorization in [None, Some("Bearer forged")] {
            let mut request = Request::post("/api/center/create_project")
                .header("x-subject", "alice")
                .header("x-author", "alice");
            if let Some(a) = authorization {
                request = request.header("authorization", a);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(r#"{"name":"forged","author":"alice"}"#))?)
                .await?;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let forged = Request::post("/api/center/create_project")
            .header("authorization", format!("Bearer {}", "a".repeat(43)))
            .body(Body::from(r#"{"name":"x","author":"forged"}"#))?;
        assert_eq!(
            app.clone().oneshot(forged).await?.status(),
            StatusCode::BAD_REQUEST
        );
        let oversized = Request::post("/api/center/create_project")
            .header("authorization", format!("Bearer {}", "a".repeat(43)))
            .body(Body::from(vec![b' '; MAX_BYTES + 1]))?;
        assert_eq!(
            app.oneshot(oversized).await?.status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        Ok(())
    }
}
