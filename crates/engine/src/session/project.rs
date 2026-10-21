use super::*;

impl ProjectRepository for SqlTransaction {
    fn member_role(&mut self, project: &str, subject: &str) -> Result<Option<Row>> {
        self.query_opt(
            &self.lock_sql(
                "SELECT m.role,p.state FROM gl_project_members m JOIN gl_projects p ON p.id=m.project WHERE m.project=$1 AND m.subject=$2 AND NOT m.removed AND p.state<>'deleted'",
                "FOR SHARE OF m",
            ),
            &[&project, &subject],
        )
    }

    fn project_head(&mut self, project: &str, lock: bool) -> Result<Option<Row>> {
        let sql = "SELECT head FROM gl_projects WHERE id=$1";
        let row = self.query_opt(
            &self.lock_sql(sql, if lock { "FOR NO KEY UPDATE" } else { "" }),
            &[&project],
        )?;
        if let Some(row) = &row {
            self.1.heads.insert(project.to_owned(), row.get(0usize)?);
        }
        Ok(row)
    }

    fn project_head_for_edit(&mut self, project: &str) -> Result<Option<Row>> {
        let row = self.query_opt(
            &self.lock_sql("SELECT head FROM gl_projects WHERE id=$1", "FOR SHARE"),
            &[&project],
        )?;
        if let Some(row) = &row {
            self.1.heads.insert(project.to_owned(), row.get(0usize)?);
        }
        Ok(row)
    }

    fn insert_project(&mut self, project: &str, name: &str) -> Result<()> {
        self.execute(
            "INSERT INTO gl_projects(id,name) VALUES($1,$2)",
            &[&project, &name],
        )
    }

    fn insert_owner(&mut self, project: &str, subject: &str) -> Result<()> {
        self.execute(
            "INSERT INTO gl_project_members(project,subject,role) VALUES($1,$2,'owner')",
            &[&project, &subject],
        )
    }

    fn list_projects(&mut self, subject: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT p.id,p.name,p.head,p.state,m.role FROM gl_projects p JOIN gl_project_members m ON m.project=p.id WHERE m.subject=$1 AND NOT m.removed AND p.state<>'deleted' AND p.id>$2 ORDER BY p.id LIMIT $3", &[&subject, &after, &limit])
    }

    fn project_info(&mut self, project: &str) -> Result<Row> {
        self.query_one(
            "SELECT name,head,state FROM gl_projects WHERE id=$1",
            &[&project],
        )
    }

    fn live_project(&mut self, project: &str, lock: bool) -> Result<Option<Row>> {
        let sql = "SELECT name,head,state FROM gl_projects WHERE id=$1 AND state<>'deleted'";
        if lock {
            self.query_opt(&self.lock_sql(sql, "FOR UPDATE"), &[&project])
        } else {
            self.query_opt(sql, &[&project])
        }
    }

    fn list_members(&mut self, project: &str, after: &str, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT subject,role FROM gl_project_members WHERE project=$1 AND NOT removed AND subject>$2 ORDER BY subject LIMIT $3", &[&project, &after, &limit])
    }

    fn remove_member(&mut self, project: &str, subject: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_project_members SET removed=true WHERE project=$1 AND subject=$2",
            &[&project, &subject],
        )
    }

    fn set_project_state(&mut self, project: &str, state: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_projects SET state=$2 WHERE id=$1",
            &[&project, &state],
        )
    }

    fn owned_projects(&mut self, subject: &str) -> Result<Row> {
        if matches!(self.0.backend, Backend::Postgis(_)) {
            // Serialize quota checks through commit, including when the subject has no projects.
            self.query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,7883))::text",
                &[&subject],
            )?;
        }
        self.query_one("SELECT count(*) FROM gl_project_members m JOIN gl_projects p ON p.id=m.project WHERE m.subject=$1 AND m.role='owner' AND NOT m.removed AND p.state<>'deleted'", &[&subject])
    }

    fn ensure_identity(&mut self, project: &str, subject: &str) -> Result<()> {
        // Audit rows reference a member row; administrators acting without
        // membership get an inactive placeholder that grants nothing.
        self.execute("INSERT INTO gl_project_members(project,subject,role,removed) VALUES($1,$2,'viewer',true) ON CONFLICT(project,subject) DO NOTHING", &[&project, &subject])
    }

    fn owner_summary(&mut self, project: &str, subject: &str) -> Result<Row> {
        self.query_one("SELECT count(*) FILTER (WHERE role='owner'), coalesce(max(CASE WHEN subject=$2 AND role='owner' THEN 1 ELSE 0 END),0) FROM gl_project_members WHERE project=$1 AND NOT removed", &[&project, &subject])
    }

    fn set_member(&mut self, project: &str, subject: &str, role: &str) -> Result<()> {
        self.execute("INSERT INTO gl_project_members(project,subject,role) VALUES($1,$2,$3) ON CONFLICT(project,subject) DO UPDATE SET role=excluded.role,removed=false", &[&project, &subject, &role])
    }

    fn rename_project(&mut self, project: &str, name: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_projects SET name=$2 WHERE id=$1",
            &[&project, &name],
        )
    }

    fn purge_data(&mut self, project: &str, dataset: Option<&str>) -> Result<()> {
        self.1.datasets.clear();
        self.release_sources(project, dataset)?;
        self.execute("INSERT INTO gl_purge VALUES($1)", &[&project])?;
        if let Some(dataset) = dataset {
            // Capture only workspaces touched by this dataset, including published drafts.
            let affected = self.query("SELECT workspace FROM gl_workspace_changes WHERE project=$1 AND dataset=$2 UNION SELECT c.workspace FROM gl_commits c JOIN gl_commit_changes d ON c.project=d.project AND c.revision=d.revision WHERE d.project=$1 AND d.dataset=$2", &[&project, &dataset])?;
            let commits = self.query(
                "SELECT DISTINCT revision FROM gl_commit_changes WHERE project=$1 AND dataset=$2",
                &[&project, &dataset],
            )?;
            for table in ["gl_workspace_changes", "gl_history", "gl_commit_changes"] {
                self.execute(
                    &format!("DELETE FROM {table} WHERE project=$1 AND dataset=$2"),
                    &[&project, &dataset],
                )?;
            }
            for row in commits {
                let revision: i64 = row.get(0usize)?;
                let remaining: i64 = self
                    .query_one(
                        "SELECT count(*) FROM gl_commit_changes WHERE project=$1 AND revision=$2",
                        &[&project, &revision],
                    )?
                    .get(0usize)?;
                if remaining == 0 {
                    let revision_text = revision.to_string();
                    self.execute("DELETE FROM gl_idempotency WHERE project=$1 AND gl_json_field(result,'revision')=$2", &[&project, &revision_text])?;
                    self.execute(
                        "DELETE FROM gl_commits WHERE project=$1 AND revision=$2",
                        &[&project, &revision],
                    )?;
                }
            }
            for row in affected {
                let workspace: String = row.get(0usize)?;
                let remaining: i64 = self.query_one("SELECT (SELECT count(*) FROM gl_workspace_changes WHERE project=$1 AND workspace=$2)+(SELECT count(*) FROM gl_commits WHERE project=$1 AND workspace=$2)", &[&project, &workspace])?.get(0usize)?;
                if remaining == 0 {
                    self.execute("DELETE FROM gl_idempotency WHERE project=$1 AND gl_json_field(payload,'workspace')=$2", &[&project, &workspace])?;
                    self.execute(
                        "DELETE FROM gl_workspaces WHERE project=$1 AND id=$2",
                        &[&project, &workspace],
                    )?;
                } else {
                    self.invalidate_resolutions(project, &workspace)?;
                    self.execute(
                        "UPDATE gl_workspaces SET version=version+1 WHERE project=$1 AND id=$2",
                        &[&project, &workspace],
                    )?;
                }
            }
            self.execute(
                "DELETE FROM gl_datasets WHERE project=$1 AND id=$2",
                &[&project, &dataset],
            )?;
        } else {
            for table in [
                "gl_workspace_changes",
                "gl_history",
                "gl_commit_changes",
                "gl_idempotency",
                "gl_commits",
                "gl_workspaces",
                "gl_datasets",
            ] {
                self.execute(
                    &format!("DELETE FROM {table} WHERE project=$1"),
                    &[&project],
                )?;
            }
        }
        // The guard is transaction-local in lifetime; errors roll it and all deletes back.
        self.execute("DELETE FROM gl_purge WHERE project=$1", &[&project])
    }
}
