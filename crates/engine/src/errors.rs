use crate::*;
pub struct Error {
    pub status: u16,
    pub body: Value,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
impl Error {
    pub(crate) fn caused_by(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        self.source = Some(Box::new(source));
        self
    }
    pub(crate) fn invalid_json(source: serde_json::Error) -> Self {
        Self::new(400, "invalid JSON request").caused_by(source)
    }
    pub(crate) fn stored_json(source: serde_json::Error) -> Self {
        Self::new(500, "invalid stored Center value").caused_by(source)
    }
    pub fn new(status: u16, message: &str) -> Self {
        let code = match status {
            400 | 422 => "invalid_argument",
            401 => "unauthenticated",
            403 => "permission_denied",
            404 => "not_found",
            409 => "conflict",
            413 => "resource_exhausted",
            429 => "busy",
            408 | 504 => "deadline_exceeded",
            503 => "unavailable",
            _ => "internal",
        };
        Self {
            status,
            body: json!({"error":{"code":code,"message":message,"request_id":Uuid::new_v4()}}),
            source: None,
        }
    }
    pub(crate) fn database(source: tokio_postgres::Error) -> Self {
        let error = if source.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) {
            Self::new(409, "resource already exists")
        } else {
            Self::new(503, "database operation failed")
        };
        // Only SQLSTATE is logged: driver messages can contain values or DSNs.
        tracing::warn!(request_id = %error.body["error"]["request_id"], sqlstate = source.code().map_or("transport", |c| c.code()), "center database failure");
        Self {
            source: Some(Box::new(source)),
            ..error
        }
    }
}
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Error")
            .field("status", &self.status)
            .field("body", &self.body)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.body)
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|e| e as _)
    }
}
impl From<tokio_postgres::Error> for Error {
    fn from(source: tokio_postgres::Error) -> Self {
        Self::database(source)
    }
}
