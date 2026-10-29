use super::Row;
use crate::Result;

/// Project lifecycle, memberships and lifecycle purges; authorization decisions stay in Application.
pub trait ProjectRepository: Send {
    /// (role, project state) for an active member of a project that is not deleted.
    fn member_role(&mut self, project: &str, subject: &str) -> Result<Option<Row>>;

    fn project_head(&mut self, project: &str, lock: bool) -> Result<Option<Row>>;

    /// Keep project state stable while independent workspaces are edited concurrently.
    fn project_head_for_edit(&mut self, project: &str) -> Result<Option<Row>> {
        self.project_head(project, true)
    }

    fn insert_project(&mut self, project: &str, name: &str) -> Result<()>;

    fn insert_owner(&mut self, project: &str, subject: &str) -> Result<()>;

    /// (project, name, head, state, role) for active memberships of projects that are not deleted.
    fn list_projects(&mut self, subject: &str, after: &str, limit: i64) -> Result<Vec<Row>>;

    /// (name, head, state).
    fn project_info(&mut self, project: &str) -> Result<Row>;

    /// (name, head, state) of a project that is not deleted; `lock` serializes lifecycle changes.
    fn live_project(&mut self, project: &str, lock: bool) -> Result<Option<Row>>;

    /// (subject, role) of active members ordered by subject.
    fn list_members(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>>;

    /// Deactivate a membership; history and audit rows keep referencing it.
    fn remove_member(&mut self, project: &str, subject: &str) -> Result<()>;

    /// One of active, archived, deleted.
    fn set_project_state(&mut self, project: &str, state: &str) -> Result<()>;

    /// Lock creation for this subject through commit and count their active owned projects.
    fn owned_projects(&mut self, subject: &str) -> Result<Row>;

    /// Ensure an (inactive, if new) member row exists so audit events can reference `subject`.
    fn ensure_identity(&mut self, project: &str, subject: &str) -> Result<()>;

    fn owner_summary(&mut self, project: &str, subject: &str) -> Result<Row>;

    fn set_member(&mut self, project: &str, subject: &str, role: &str) -> Result<()>;

    fn rename_project(&mut self, project: &str, name: &str) -> Result<()>;

    /// Purge the selected dataset, or all project data. Caller holds the project lock.
    /// Shared commits/workspaces retain other datasets; audit records remain immutable.
    fn purge_data(&mut self, project: &str, dataset: Option<&str>) -> Result<()>;
}
