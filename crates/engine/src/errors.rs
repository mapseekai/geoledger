use crate::*;
pub struct Error {
    pub status: u16,
    pub body: Value,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
impl Error {
    pub fn caused_by(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
    pub(crate) fn invalid_json(source: serde_json::Error) -> Self {
        Self::new(400, "invalid JSON request").caused_by(source)
    }
    pub(crate) fn stored_json(source: serde_json::Error) -> Self {
        Self::new(500, "invalid stored GeoLedger value").caused_by(source)
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
        } else if source.code() == Some(&tokio_postgres::error::SqlState::QUERY_CANCELED) {
            // Cancellation source is classified by the executing session, using
            // its deadline, not a locale-dependent PostgreSQL message string.
            let mut error = Self::new(409, "database operation cancelled");
            error.body["error"]["code"] = json!("cancelled");
            error
        } else if source.code() == Some(&tokio_postgres::error::SqlState::LOCK_NOT_AVAILABLE) {
            Self::new(429, "database lock unavailable")
        } else {
            Self::new(503, "database operation failed")
        };
        Self {
            source: Some(Box::new(source)),
            ..error
        }
    }
    /// The request ID returned to the client in the error body.
    pub fn request_id(&self) -> Option<&str> {
        self.body["error"]["request_id"].as_str()
    }
    /// Replace the body request ID with the transport request ID so logs and
    /// responses correlate.
    pub fn with_request_id(mut self, id: &str) -> Self {
        if let Some(e) = self.body.get_mut("error").and_then(Value::as_object_mut) {
            e.insert("request_id".into(), Value::String(id.to_owned()));
        }
        self
    }
    /// A log-safe description of the underlying cause. PostgreSQL errors are
    /// reduced to their SQLSTATE because driver messages can contain values or DSNs.
    pub fn diagnostic(&self) -> Option<String> {
        let source = self.source.as_deref()?;
        if let Some(pg) = source.downcast_ref::<tokio_postgres::Error>() {
            return Some(format!(
                "postgres sqlstate={}",
                pg.code().map_or("transport", |c| c.code())
            ));
        }
        if let Some(json) = source.downcast_ref::<serde_json::Error>() {
            // serde messages can quote stored values; keep only the category and position.
            return Some(format!(
                "json {:?} error at line {} column {}",
                json.classify(),
                json.line(),
                json.column()
            ));
        }
        let mut text = source.to_string();
        if text.len() > 512 {
            let mut end = 512;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        Some(text)
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_omit_values_and_request_ids_are_replaceable() {
        let Err(source) = serde_json::from_str::<u8>("\"secret-value\"") else {
            panic!("parse unexpectedly succeeded");
        };
        let error = Error::new(500, "x").caused_by(source);
        let text = error.diagnostic().unwrap_or_default();
        assert!(text.starts_with("json"), "{text}");
        assert!(!text.contains("secret-value"), "{text}");
        let error = error.with_request_id("gw-1");
        assert_eq!(error.request_id(), Some("gw-1"));
        assert!(Error::new(404, "x").diagnostic().is_none());
    }
}
