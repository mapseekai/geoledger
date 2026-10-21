//! Backend extension contract. All capabilities borrow one transaction and share its snapshot,
//! locks, deadline and response budget. Implementations roll back on Drop; only the owner commits.
mod audit;
mod authentication;
mod backend;
mod dataset;
mod history;
mod project;
mod publication;
mod query;
mod row;
mod tables;
mod workspace;

use crate::Result;
pub use audit::AuditRepository;
pub use authentication::CredentialRepository;
pub use backend::{StorageBackend, StorageMaintenance};
pub use dataset::DatasetRepository;
pub use history::HistoryRepository;
pub use project::ProjectRepository;
pub use publication::{ConflictQuery, PublicationRepository};
pub use query::{FeatureQuery, FeatureRepository};
pub use row::{Cell, Decode, Row};
pub use tables::PostgisTables;
pub use workspace::{DraftDelta, WorkspaceRepository};

/// Encoded response budget shared by every capability of a transaction.
pub trait ResponseBudget: Send {
    fn set_response_limit(&mut self, bytes: usize);

    fn response_limit(&self) -> usize;

    fn set_response_reservation(&mut self, reservation: Option<crate::ResponseReservation>);

    fn response_reservation(&self) -> Option<crate::ResponseReservation>;
}

/// Transaction ownership and atomic commit. Domain handlers take only the capabilities they use.
/// Every supertrait must operate on this same transaction, including staging, receipts and audit.
pub trait RepositoryTransaction:
    ResponseBudget
    + CredentialRepository
    + ProjectRepository
    + DatasetRepository
    + WorkspaceRepository
    + FeatureRepository
    + HistoryRepository
    + PublicationRepository
    + AuditRepository
    + PostgisTables
{
    fn commit(self: Box<Self>) -> Result<()>;
}
