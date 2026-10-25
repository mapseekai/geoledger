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
        let mut after = String::new();
        loop {
            let rows = self.query("SELECT id FROM gl_datasets WHERE project=$1 AND (CAST($2 AS text) IS NULL OR id=$2) AND id>$3 AND storage_table IS NOT NULL ORDER BY id LIMIT 32", &[&project, &dataset, &after])?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let id: String = row.get(0usize)?;
                self.drop_dataset_table(project, &id)?;
                after = id;
            }
        }
        self.execute("INSERT INTO gl_purge VALUES($1)", &[&project])?;
        if let Some(dataset) = dataset {
            // Capture schema-only drafts and publications before deleting this dataset's state.
            let affected = self.query("SELECT workspace FROM gl_workspace_changes WHERE project=$1 AND dataset=$2 UNION SELECT workspace FROM gl_workspace_schemas WHERE project=$1 AND dataset=$2 UNION SELECT c.workspace FROM gl_commits c JOIN gl_commit_changes d ON c.project=d.project AND c.revision=d.revision WHERE d.project=$1 AND d.dataset=$2 UNION SELECT c.workspace FROM gl_commits c JOIN gl_schema_history h ON c.project=h.project AND c.revision=h.revision WHERE h.project=$1 AND h.dataset=$2 UNION SELECT c.workspace FROM gl_commits c JOIN gl_datasets d ON d.project=c.project AND d.source_revision=c.revision WHERE d.project=$1 AND d.id=$2", &[&project, &dataset])?;
            let commits = self.query(
                "SELECT DISTINCT revision FROM gl_commit_changes WHERE project=$1 AND dataset=$2 UNION SELECT revision FROM gl_schema_history WHERE project=$1 AND dataset=$2 AND revision>0 UNION SELECT source_revision FROM gl_datasets WHERE project=$1 AND id=$2 AND source_revision>0",
                &[&project, &dataset],
            )?;
            // Remove only the target dataset's schema rows while the purge guard is active.
            // Decide whether commits/workspaces survive before any dataset FK can cascade.
            for table in [
                "gl_workspace_schemas",
                "gl_schema_history",
                "gl_workspace_changes",
                "gl_history",
                "gl_commit_changes",
            ] {
                self.execute(
                    &format!("DELETE FROM {table} WHERE project=$1 AND dataset=$2"),
                    &[&project, &dataset],
                )?;
            }
            if matches!(self.0.backend, Backend::Postgis(_)) {
                self.execute(
                    "DELETE FROM gl_source_versions WHERE project=$1 AND dataset=$2",
                    &[&project, &dataset],
                )?;
            }
            for row in commits {
                let revision: i64 = row.get(0usize)?;
                let remaining: i64 = self
                    .query_one(
                        "SELECT (SELECT count(*) FROM gl_commit_changes WHERE project=$1 AND revision=$2)+(SELECT count(*) FROM gl_schema_history WHERE project=$1 AND revision=$2)",
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
                let remaining: i64 = self.query_one("SELECT (SELECT count(*) FROM gl_workspace_changes WHERE project=$1 AND workspace=$2)+(SELECT count(*) FROM gl_workspace_schemas WHERE project=$1 AND workspace=$2)+(SELECT count(*) FROM gl_commits WHERE project=$1 AND workspace=$2)", &[&project, &workspace])?.get(0usize)?;
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
            if matches!(self.0.backend, Backend::Postgis(_)) {
                self.execute(
                    "DELETE FROM gl_source_versions WHERE project=$1",
                    &[&project],
                )?;
            }
            for table in [
                "gl_workspace_schemas",
                "gl_schema_history",
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test fixture setup and assertions fail immediately"
)]
mod tests {
    use crate::{Application, Storage};
    use serde_json::{Value, json};
    use uuid::Uuid;

    struct Fixture {
        _dir: tempfile::TempDir,
        app: Application,
        path: std::path::PathBuf,
        project: Value,
        primary: Value,
        survivor: Value,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("purge.db");
            let app = Application::new(Storage::Sqlite(path.clone()));
            app.migrate().unwrap();
            let project = app
                .execute("alice", "create_project", json!({"name":"schema purge"}))
                .unwrap()["project"]
                .clone();
            let dataset = |name| {
                app.execute(
                    "alice",
                    "create_dataset",
                    json!({"project":project,"name":name,"geometry_type":"point"}),
                )
                .unwrap()["dataset"]
                    .clone()
            };
            let primary = dataset("primary");
            let survivor = dataset("survivor");
            Self {
                _dir: dir,
                app,
                path,
                project,
                primary,
                survivor,
            }
        }

        fn call(&self, operation: &str, mut input: Value) -> Value {
            input["project"] = self.project.clone();
            self.app.execute("alice", operation, input).unwrap()
        }

        fn workspace(&self) -> Value {
            self.call("create_workspace", json!({}))["workspace"].clone()
        }

        fn request(&self, workspace: &Value, version: i64) -> Value {
            json!({"project":self.project,"workspace":workspace,"expected_workspace_version":version,"request_id":Uuid::new_v4(),"message":"schema purge fixture"})
        }

        fn export(&self) -> Vec<u8> {
            let mut data = Vec::new();
            self.app.export_data(&mut data).unwrap();
            data
        }

        fn remove_primary(&self) {
            self.call(
                "delete_dataset",
                json!({"dataset":self.primary,"confirm_name":"primary"}),
            );
        }
    }

    fn add(dataset: &Value, kind: &str) -> Value {
        json!({"dataset":dataset,"add":[{"name":"shared","type":kind}]})
    }

    fn edit(dataset: &Value) -> Value {
        json!({"dataset":dataset,"feature_id":"only","feature":{"type":"Feature","id":"only","properties":{"n":1},"geometry":null}})
    }

    #[test]
    fn schema_purge_preserves_other_dataset_draft() {
        let f = Fixture::new();
        let workspace = f.workspace();
        f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"schema_edits":[add(&f.survivor,"string")],"edits":[edit(&f.primary)]}));
        f.remove_primary();
        let state = f.call("get_workspace", json!({"workspace":workspace}));
        assert_eq!(state["version"], 2);
        assert_eq!(state["status"], "open");
        let schema = f.call(
            "schema",
            json!({"dataset":f.survivor,"workspace":workspace}),
        );
        assert_eq!(schema["fields"][0]["name"], "shared");
        assert_eq!(schema["fields"][0]["type"], "text");
        let diff = f.call("diff", json!({"workspace":workspace}));
        assert_eq!(diff["changes"], json!([]));
        assert_eq!(diff["schema_changes"].as_array().unwrap().len(), 1);
        assert_eq!(diff["schema_changes"][0]["dataset"], f.survivor);
        let result = f.call("publish", f.request(&workspace, 2));
        assert_eq!(result["revision"], 1);
        assert_eq!(
            f.call("schema", json!({"dataset":f.survivor}))["fields"],
            schema["fields"]
        );
    }

    #[test]
    fn schema_purge_advances_schema_only_workspace_cas() {
        let f = Fixture::new();
        let workspace = f.workspace();
        f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"schema_edits":[add(&f.primary,"integer"),add(&f.survivor,"string")]}));
        f.remove_primary();
        assert_eq!(
            f.call("get_workspace", json!({"workspace":workspace}))["version"],
            2
        );
        let before = f.export();
        let stale = f.app.execute("alice", "save", json!({"project":f.project,"workspace":workspace,"expected_workspace_version":1,"schema_edits":[]})).unwrap_err();
        assert_eq!(stale.status, 409);
        assert_eq!(f.export(), before);
        let diff = f.call("diff", json!({"workspace":workspace}));
        assert_eq!(diff["schema_changes"].as_array().unwrap().len(), 1);
        assert_eq!(diff["schema_changes"][0]["after"][0]["type"], "text");
        f.call(
            "save",
            json!({"workspace":workspace,"expected_workspace_version":2,"schema_edits":[]}),
        );
        assert_eq!(
            f.call("get_workspace", json!({"workspace":workspace}))["version"],
            3
        );
        assert_eq!(
            f.call("diff", json!({"workspace":workspace}))["schema_changes"],
            json!([])
        );
    }

    #[test]
    fn schema_purge_preserves_mixed_commit_receipt_and_restore() {
        for schema_only in [false, true] {
            let f = Fixture::new();
            let workspace = f.workspace();
            let schemas = if schema_only {
                vec![add(&f.primary, "integer"), add(&f.survivor, "string")]
            } else {
                vec![add(&f.survivor, "string")]
            };
            let edits = if schema_only {
                vec![]
            } else {
                vec![edit(&f.primary)]
            };
            f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"schema_edits":schemas,"edits":edits}));
            let request = f.request(&workspace, 1);
            let receipt = f.call("publish", request.clone());
            let surviving_schema = f.call("schema", json!({"dataset":f.survivor,"revision":1}));
            f.remove_primary();
            let history = f.call("history", json!({}));
            assert_eq!(history["commits"].as_array().unwrap().len(), 1);
            let commit = f.call("commit", json!({"revision":1}));
            assert_eq!(commit["changes"], json!([]));
            assert_eq!(commit["schema_changes"].as_array().unwrap().len(), 1);
            assert_eq!(commit["schema_changes"][0]["dataset"], f.survivor);
            assert_eq!(commit["schema_changes"][0]["after"][0]["type"], "text");
            assert_eq!(
                f.call("get_workspace", json!({"workspace":workspace}))["version"],
                3
            );
            assert_eq!(f.call("publish", request.clone()), receipt);
            assert_eq!(
                f.call(
                    "schema",
                    json!({"dataset":f.survivor,"workspace":workspace})
                )["fields"],
                surviving_schema["fields"]
            );
            let inverse = f.call("restore", json!({"revision":1}))["workspace"].clone();
            let diff = f.call("diff", json!({"workspace":inverse}));
            assert_eq!(diff["schema_changes"].as_array().unwrap().len(), 1);
            assert_eq!(diff["schema_changes"][0]["dataset"], f.survivor);
            assert_eq!(diff["schema_changes"][0]["after"], json!([]));
            f.call("publish", f.request(&inverse, 0));
            assert_eq!(
                f.call("schema", json!({"dataset":f.survivor}))["fields"],
                json!([])
            );
            assert_eq!(
                f.call("schema", json!({"dataset":f.survivor,"revision":1})),
                surviving_schema
            );
            assert_eq!(f.call("publish", request), receipt);
        }
    }

    #[test]
    fn schema_purge_preserves_other_dataset_feature_commit() {
        let f = Fixture::new();
        let workspace = f.workspace();
        f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"schema_edits":[add(&f.primary,"integer")],"edits":[edit(&f.survivor)]}));
        let request = f.request(&workspace, 1);
        let receipt = f.call("publish", request.clone());
        f.remove_primary();
        let commit = f.call("commit", json!({"revision":1}));
        assert_eq!(commit["schema_changes"], json!([]));
        assert_eq!(commit["changes"].as_array().unwrap().len(), 1);
        assert_eq!(commit["changes"][0]["dataset"], f.survivor);
        assert_eq!(f.call("publish", request), receipt);
        let inverse = f.call("restore", json!({"revision":1}))["workspace"].clone();
        f.call("publish", f.request(&inverse, 0));
        assert_eq!(
            f.call("features", json!({"dataset":f.survivor}))["features"],
            json!([])
        );
    }

    #[test]
    fn schema_purge_removes_only_empty_schema_workspaces_and_commits() {
        for published in [false, true] {
            let f = Fixture::new();
            let workspace = f.workspace();
            let untouched = f.workspace();
            f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"schema_edits":[add(&f.primary,"integer")]}));
            let request = f.request(&workspace, 1);
            if published {
                f.call("publish", request.clone());
            }
            f.remove_primary();
            assert_eq!(
                f.app
                    .execute(
                        "alice",
                        "get_workspace",
                        json!({"project":f.project,"workspace":workspace})
                    )
                    .unwrap_err()
                    .status,
                404
            );
            assert_eq!(
                f.call("get_workspace", json!({"workspace":untouched}))["version"],
                0
            );
            assert_eq!(f.call("history", json!({}))["commits"], json!([]));
            if published {
                assert_eq!(
                    f.app
                        .execute("alice", "publish", request)
                        .unwrap_err()
                        .status,
                    404
                );
            }
            let db = rusqlite::Connection::open(&f.path).unwrap();
            for table in [
                "gl_workspace_schemas",
                "gl_schema_history",
                "gl_commits",
                "gl_idempotency",
                "gl_purge",
            ] {
                assert_eq!(
                    db.query_row(
                        &format!("SELECT count(*) FROM {table} WHERE project=?1"),
                        [f.project.as_str().unwrap()],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                    0,
                    "{table}"
                );
            }
            let replacement = f.call(
                "create_dataset",
                json!({"name":"primary","geometry_type":"point"}),
            )["dataset"]
                .clone();
            assert_eq!(
                f.call("schema", json!({"dataset":replacement}))["fields"],
                json!([])
            );
        }
    }

    #[test]
    fn schema_purge_rolls_back_and_cleans_project_with_immutable_history() {
        for operation in ["delete_dataset", "delete_project"] {
            let f = Fixture::new();
            let workspace = f.workspace();
            f.call("save", json!({"workspace":workspace,"expected_workspace_version":0,"schema_edits":[add(&f.primary,"integer"),add(&f.survivor,"string")]}));
            let request = f.request(&workspace, 1);
            let receipt = f.call("publish", request.clone());
            let draft = f.workspace();
            f.call("save", json!({"workspace":draft,"expected_workspace_version":0,"schema_edits":[{"dataset":f.primary,"drop":["shared"]},{"dataset":f.survivor,"add":[{"name":"other","type":"boolean"}]}]}));
            let db = rusqlite::Connection::open(&f.path).unwrap();
            assert!(
                db.execute(
                    "DELETE FROM gl_schema_history WHERE project=?1",
                    [f.project.as_str().unwrap()]
                )
                .is_err()
            );
            assert!(
                db.execute(
                    "UPDATE gl_schema_history SET definition='{}' WHERE project=?1",
                    [f.project.as_str().unwrap()]
                )
                .is_err()
            );
            db.execute_batch(&format!("CREATE TABLE purge_fault(enabled integer NOT NULL); INSERT INTO purge_fault VALUES(1); CREATE TRIGGER purge_audit_fault BEFORE INSERT ON gl_audit_events WHEN NEW.project='{}' AND NEW.action='{operation}' AND (SELECT enabled FROM purge_fault)=1 BEGIN SELECT RAISE(ABORT,'schema purge rollback fixture'); END;",f.project.as_str().unwrap())).unwrap();
            let body = if operation == "delete_dataset" {
                json!({"project":f.project,"dataset":f.primary,"confirm_name":"primary"})
            } else {
                json!({"project":f.project,"confirm_name":"schema purge"})
            };
            let before = f.export();
            assert_eq!(
                f.app
                    .execute("alice", operation, body.clone())
                    .unwrap_err()
                    .status,
                503
            );
            assert_eq!(f.export(), before);
            assert_eq!(f.call("publish", request.clone()), receipt);
            assert!(
                db.execute(
                    "DELETE FROM gl_schema_history WHERE project=?1",
                    [f.project.as_str().unwrap()]
                )
                .is_err()
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM gl_purge", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
            db.execute("UPDATE purge_fault SET enabled=0", []).unwrap();
            f.call(operation, body);
            if operation == "delete_dataset" {
                assert_eq!(f.call("publish", request), receipt);
                assert_eq!(
                    f.call("get_workspace", json!({"workspace":draft}))["version"],
                    2
                );
                assert_eq!(
                    f.call("schema", json!({"dataset":f.survivor,"workspace":draft}))["fields"]
                        .as_array()
                        .unwrap()
                        .len(),
                    2
                );
                assert!(
                    db.execute(
                        "DELETE FROM gl_schema_history WHERE project=?1",
                        [f.project.as_str().unwrap()]
                    )
                    .is_err()
                );
            } else {
                for table in [
                    "gl_workspace_schemas",
                    "gl_schema_history",
                    "gl_workspace_changes",
                    "gl_history",
                    "gl_commit_changes",
                    "gl_idempotency",
                    "gl_commits",
                    "gl_workspaces",
                    "gl_datasets",
                    "gl_purge",
                ] {
                    assert_eq!(
                        db.query_row(
                            &format!("SELECT count(*) FROM {table} WHERE project=?1"),
                            [f.project.as_str().unwrap()],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                        0,
                        "{table}"
                    );
                }
                assert!(
                    db.query_row(
                        "SELECT count(*) FROM gl_audit_events WHERE project=?1",
                        [f.project.as_str().unwrap()],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap()
                        > 0
                );
                assert!(
                    db.execute(
                        "DELETE FROM gl_audit_events WHERE project=?1",
                        [f.project.as_str().unwrap()]
                    )
                    .is_err()
                );
            }
        }
    }
}
