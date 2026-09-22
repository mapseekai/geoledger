//! Transport- and database-independent versioning primitives.
pub mod adapter;
pub mod graph;
pub mod merge;
pub mod model;
pub mod object;
pub mod schema;
pub mod tree;

pub use model::*;
pub use object::{ObjectId, ObjectStore, load, save};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy)]
pub enum BackendKind {
    Storage,
    Database,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("working copy has uncommitted changes")]
    Dirty,
    #[error("repository is busy; retry after the active operation finishes")]
    Busy,
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("recovery required: {0}")]
    Recovery(String),
    #[error("{message}")]
    Backend {
        kind: BackendKind,
        message: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn storage_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Backend {
            kind: BackendKind::Storage,
            message: message.into(),
            source: Box::new(source),
        }
    }
    pub fn database_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Backend {
            kind: BackendKind::Database,
            message: message.into(),
            source: Box::new(source),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) | Self::Json(_) => "invalid_argument",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Dirty => "dirty_working_copy",
            Self::Busy => "busy",
            Self::Unsupported(_) => "unsupported",
            Self::Recovery(_) => "recovery_required",
            Self::Backend {
                kind: BackendKind::Database,
                ..
            }
            | Self::Database(_) => "database_error",
            Self::Backend {
                kind: BackendKind::Storage,
                ..
            }
            | Self::Storage(_)
            | Self::Io(_) => "storage_error",
        }
    }
}
