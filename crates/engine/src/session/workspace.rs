use super::*;

impl WorkspaceRepository for SqlTransaction {
    fn workspace_state(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        write: bool,
    ) -> Result<Option<Row>> {
        self.query_opt(&self.lock_sql("SELECT base_revision,version,status FROM gl_workspaces WHERE project=$1 AND id=$2 AND owner=$3",if write {"FOR UPDATE"}else{"FOR SHARE"}), &[&project,&workspace,&subject])
    }

    fn workspace_publication_revision(
        &mut self,
        project: &str,
        workspace: &str,
    ) -> Result<Option<i64>> {
        self.query_opt(
            "SELECT revision FROM gl_commits WHERE project=$1 AND workspace=$2",
            &[&project, &workspace],
        )?
        .map(|row| row.get(0usize))
        .transpose()
    }

    fn advance_workspace(&mut self, project: &str, workspace: &str, status: &str) -> Result<Row> {
        self.query_one("UPDATE gl_workspaces SET version=version+1,status=$3 WHERE project=$1 AND id=$2 RETURNING version", &[&project, &workspace, &status])
    }

    fn insert_workspace(
        &mut self,
        project: &str,
        workspace: &str,
        subject: &str,
        base: i64,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO gl_workspaces(project,id,owner,base_revision) VALUES($1,$2,$3,$4)",
            &[&project, &workspace, &subject, &base],
        )
    }

    fn list_workspaces(
        &mut self,
        project: &str,
        subject: &str,
        after: &str,
        limit: i64,
    ) -> Result<Vec<Row>> {
        self.query("SELECT id,base_revision,version,status FROM gl_workspaces WHERE project=$1 AND owner=$2 AND id>$3 ORDER BY id LIMIT $4", &[&project, &subject, &after, &limit])
    }

    fn put_deltas(
        &mut self,
        project: &str,
        workspace: &str,
        deltas: &[crate::repository::DraftDelta],
    ) -> Result<()> {
        if deltas.is_empty() {
            return Ok(());
        }
        let updates = self
            .geometry_columns()
            .split(',')
            .map(|c| format!("{c}=excluded.{c}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut params: Vec<&dyn Parameter> = vec![&project, &workspace];
        let mut values = Vec::with_capacity(deltas.len());
        for (i, delta) in deltas.iter().enumerate() {
            let n = i * 4 + 3;
            values.push(format!(
                "($1,$2,${n},${},${},${},{})",
                n + 1,
                n + 2,
                n + 3,
                self.geometry_values(&format!("${}", n + 3))
            ));
            params.extend([
                &delta.dataset as &dyn Parameter,
                &delta.key,
                &delta.properties,
                &delta.geometry,
            ]);
        }
        self.execute(&format!("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geometry_json,{columns}) VALUES {values} ON CONFLICT(project,workspace,dataset,feature_id) DO UPDATE SET properties=excluded.properties,geometry_json=excluded.geometry_json,{updates}", columns=self.geometry_columns(), values=values.join(",")), &params)
    }

    fn invalidate_resolutions(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute("UPDATE gl_workspace_changes SET resolution_stale=true WHERE project=$1 AND workspace=$2 AND resolved_head IS NOT NULL AND NOT resolution_stale", &[&project, &workspace])
    }

    fn edit_base(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        base: i64,
    ) -> Result<Row> {
        let value = super::query::version_cells(FeatureRepository::feature_at(
            self, project, dataset, key, base,
        )?);
        let draft = self.query_opt(&format!("SELECT properties,geometry_json,resolved_head IS NOT NULL FROM {} WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4",self.workspace_key_table()), &[&project,&workspace,&dataset,&key])?;
        let resolved = draft
            .as_ref()
            .map(|r| r.get::<_, bool>(2usize))
            .transpose()?
            .unwrap_or(false);
        Ok(Row::new(vec![
            Cell::Bool(resolved),
            value[0].clone(),
            value[1].clone(),
            draft.as_ref().map_or(Cell::Null, |r| r.0[0].clone()),
            draft.as_ref().map_or(Cell::Null, |r| r.0[1].clone()),
            Cell::Bool(draft.is_some()),
        ]))
    }

    fn remove_delta(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
    ) -> Result<()> {
        self.execute(
            &format!(
                "DELETE FROM {} WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4",
                self.workspace_key_table()
            ),
            &[&project, &workspace, &dataset, &key],
        )
    }

    fn count_deltas(&mut self, project: &str, workspace: &str) -> Result<Row> {
        self.query_one(
            "SELECT coalesce((SELECT changes FROM gl_workspace_sizes WHERE project=$1 AND workspace=$2),0)",
            &[&project, &workspace],
        )
    }

    fn workspace_summary(&mut self, project: &str, workspace: &str, base: i64) -> Result<Vec<Row>> {
        let (status, revision) = self.workspace_terminal(project, workspace)?;
        if status == "discarded" {
            return Ok(Vec::new());
        }
        let (changes, scope, draft) = if status == "published" {
            if revision.is_none() {
                return Err(Error::new(500, "missing workspace publication"));
            }
            // Workspace Diff compares with its base, not the commit's preceding HEAD.
            (
                "gl_commit_changes",
                "c.revision=(SELECT revision FROM gl_commits WHERE project=$1 AND workspace=$2)",
                "c.has_after",
            )
        } else {
            (
                "gl_workspace_changes",
                "c.workspace=$2",
                "c.properties IS NOT NULL",
            )
        };
        let mut result = Vec::new();
        let datasets=self.query(&format!("SELECT DISTINCT c.dataset,d.name FROM {changes} c JOIN gl_datasets d ON d.project=c.project AND d.id=c.dataset WHERE c.project=$1 AND {scope} ORDER BY c.dataset"),&[&project,&workspace])?;
        for dataset in datasets {
            let id: String = dataset.get(0usize)?;
            let name: String = dataset.get(1usize)?;
            let metadata = self
                .dataset_exists(project, &id)?
                .ok_or_else(|| Error::new(500, "missing dataset"))?;
            let current = if let Some(table) = metadata.get::<_, Option<String>>(5usize)? {
                super::dataset::check_table_name(&table)?;
                format!(
                    "EXISTS(SELECT 1 FROM {table} t WHERE t.feature_id=c.feature_id AND t.valid_from<=$3)"
                )
            } else {
                let source: String = metadata
                    .get::<_, Option<String>>(3usize)?
                    .ok_or_else(|| Error::new(500, "missing source"))?;
                let source: super::tables::Source = crate::codec::stored(&source)?;
                self.ensure_source_clean(project, &id, &source)?;
                let table = super::tables::relation(&source.table);
                let key = super::tables::quote(&source.table.id_column);
                let column = source.table.id_column.replace('\'', "''");
                let start: i64 = metadata.get(6usize)?;
                format!(
                    "EXISTS(SELECT 1 FROM {table} t LEFT JOIN gl_source_versions v ON v.project=$1 AND v.dataset=$4 AND v.feature_id=t.{key}::text COLLATE \"C\" WHERE t.{key}=(jsonb_populate_record(NULL::{table},jsonb_build_object('{column}',c.feature_id))).{key} AND coalesce(v.valid_from,{start})<=$3)"
                )
            };
            let row=self.query_one(&format!("WITH changes AS MATERIALIZED (SELECT c.feature_id,{draft} AS draft,(EXISTS(SELECT 1 FROM gl_history h WHERE h.project=$1 AND h.dataset=$4 AND h.feature_id=c.feature_id AND h.valid_from<=$3 AND h.valid_to>$3 AND h.properties IS NOT NULL) OR {current}) AS before FROM {changes} c WHERE c.project=$1 AND {scope} AND c.dataset=$4) SELECT coalesce(sum(CASE WHEN NOT before AND draft THEN 1 ELSE 0 END),0),coalesce(sum(CASE WHEN before AND NOT draft THEN 1 ELSE 0 END),0),coalesce(sum(CASE WHEN before AND draft THEN 1 ELSE 0 END),0) FROM changes"), &[&project,&workspace,&base,&id])?;
            let added: i64 = row.get(0usize)?;
            let deleted: i64 = row.get(1usize)?;
            let modified: i64 = row.get(2usize)?;
            if added + deleted + modified > 0 {
                result.push(Row::new(vec![
                    Cell::Text(id),
                    Cell::Text(name),
                    Cell::Integer(added),
                    Cell::Integer(deleted),
                    Cell::Integer(modified),
                ]));
            }
        }
        Ok(result)
    }

    fn diff_page(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>> {
        let (status, revision) = self.workspace_terminal(project, workspace)?;
        if status == "discarded" {
            return Ok(Vec::new());
        }
        if status == "published" {
            let Some(revision) = revision else {
                return Ok(Vec::new());
            };
            let keys = self.query("SELECT dataset,feature_id FROM gl_commit_changes WHERE project=$1 AND revision=$2 AND (dataset,feature_id)>($3,$4) ORDER BY dataset,feature_id LIMIT $5", &[&project,&revision,&after_dataset,&after_key,&limit])?;
            let mut rows = Vec::with_capacity(keys.len());
            for key in keys {
                let dataset: String = key.get(0usize)?;
                let feature_id: String = key.get(1usize)?;
                let published = super::query::version_cells(FeatureRepository::feature_at(
                    self,
                    project,
                    &dataset,
                    &feature_id,
                    revision,
                )?);
                let original = super::query::version_cells(FeatureRepository::feature_at(
                    self,
                    project,
                    &dataset,
                    &feature_id,
                    base,
                )?);
                let mut row = Row::new(vec![Cell::Text(dataset), Cell::Text(feature_id)]);
                row.0.extend(published);
                row.0.extend(original);
                rows.push(row);
            }
            return Ok(rows);
        }
        let mut rows = self.query("SELECT dataset,feature_id,properties,geometry_json FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($3,$4) ORDER BY dataset,feature_id LIMIT $5", &[&project,&workspace,&after_dataset,&after_key,&limit])?;
        for row in &mut rows {
            let dataset: String = row.get(0usize)?;
            let key: String = row.get(1usize)?;
            row.0
                .extend(super::query::version_cells(FeatureRepository::feature_at(
                    self, project, &dataset, &key, base,
                )?));
        }
        Ok(rows)
    }

    fn diff_has_after(
        &mut self,
        project: &str,
        workspace: &str,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<bool> {
        let (status, revision) = self.workspace_terminal(project, workspace)?;
        if status == "discarded" {
            return Ok(false);
        }
        if status == "published" {
            return if let Some(revision) = revision {
                self.query_one("SELECT EXISTS(SELECT 1 FROM gl_commit_changes WHERE project=$1 AND revision=$2 AND (dataset,feature_id)>($3,$4))", &[&project,&revision,&after_dataset,&after_key])?.get(0usize)
            } else {
                Ok(false)
            };
        }
        self.query_one("SELECT EXISTS(SELECT 1 FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($3,$4))", &[&project, &workspace, &after_dataset, &after_key])?.get(0usize)
    }

    fn mark_resolved(
        &mut self,
        project: &str,
        workspace: &str,
        dataset: &str,
        key: &str,
        head: i64,
    ) -> Result<()> {
        self.execute(&format!("UPDATE {} SET resolved_head=$5,resolution_stale=false WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4", self.workspace_key_table()), &[&project, &workspace, &dataset, &key, &head])
    }

    fn clear_deltas(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.execute(
            &format!(
                "DELETE FROM {} WHERE project=$1 AND workspace=$2",
                self.workspace_key_table()
            ),
            &[&project, &workspace],
        )
    }

    fn advance_base(&mut self, project: &str, workspace: &str, revision: i64) -> Result<()> {
        self.execute(
            "UPDATE gl_workspaces SET base_revision=$3 WHERE project=$1 AND id=$2",
            &[&project, &workspace, &revision],
        )
    }
}

impl SqlTransaction {
    fn workspace_terminal(
        &mut self,
        project: &str,
        workspace: &str,
    ) -> Result<(String, Option<i64>)> {
        let row = self.query_one("SELECT w.status,(SELECT c.revision FROM gl_commits c WHERE c.project=w.project AND c.workspace=w.id) FROM gl_workspaces w WHERE w.project=$1 AND w.id=$2", &[&project,&workspace])?;
        Ok((row.get(0usize)?, row.get(1usize)?))
    }

    pub(super) fn workspace_key_table(&self) -> &'static str {
        match self.0.backend {
            // ANALYZE on a new database can retain a one-row estimate after drafts grow.
            Backend::Sqlite(_) => {
                "gl_workspace_changes INDEXED BY sqlite_autoindex_gl_workspace_changes_1"
            }
            Backend::Postgis(_) => "gl_workspace_changes",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Application, DatasetSchema, SchemaField};
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn save_schema_reads_scale_with_datasets_including_missing_definitions() -> Result<()> {
        let dir =
            tempfile::tempdir().map_err(|e| Error::new(500, "test directory").caused_by(e))?;
        let storage = Storage::Sqlite(dir.path().join("save-schema-reads.db"));
        let app = Application::new(storage.clone());
        app.migrate()?;
        let project =
            app.execute("alice", "create_project", json!({"name":"schema reads"}))?["project"]
                .clone();
        let call = |operation: &str, mut input: Value| {
            input["project"] = project.clone();
            app.execute("alice", operation, input)
        };
        let mut datasets = Vec::new();
        for name in ["first", "second"] {
            datasets.push(
                call(
                    "create_dataset",
                    json!({"name":name,"geometry_type":"point"}),
                )?["dataset"]
                    .as_str()
                    .ok_or_else(crate::bad)?
                    .to_owned(),
            );
        }
        let project = project.as_str().ok_or_else(crate::bad)?;
        let workspace = call("create_workspace", json!({}))?["workspace"]
            .as_str()
            .ok_or_else(crate::bad)?
            .to_owned();
        let definition = serde_json::to_string(&DatasetSchema {
            supported: true,
            fields: vec![SchemaField {
                name: "label".into(),
                kind: "text".into(),
                nullable: true,
                editable: true,
            }],
            ..Default::default()
        })
        .map_err(Error::stored_json)?;
        let pool = Arc::new(postgres::Pool::default());
        for source in ["missing", "history", "workspace"] {
            for dataset_count in [1, 2] {
                for edit_count in [100, dataset_count] {
                    let mut t = Client::open(&storage, &pool, Duration::from_secs(30))?
                        .transaction(false)?;
                    for dataset in &datasets[..dataset_count] {
                        match source {
                            "history" => t.execute("INSERT INTO gl_schema_history(project,dataset,revision,definition) VALUES($1,$2,0,$3)", &[&project,&dataset,&definition])?,
                            "workspace" => t.execute("INSERT INTO gl_workspace_schemas(project,workspace,dataset,definition) VALUES($1,$2,$3,$4)", &[&project,&workspace,&dataset,&definition])?,
                            _ => {}
                        }
                    }
                    let reads = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
                    let observed = reads.clone();
                    let Backend::Sqlite(db) = &t.0.backend else {
                        return Err(Error::new(500, "SQLite fixture required"));
                    };
                    {
                        let db = db.read().map_err(sqlite_error)?;
                        // Authorizer observes preparation; disable caching so every execution is counted.
                        db.set_prepared_statement_cache_capacity(0);
                        db.authorizer(Some(move |context: AuthContext<'_>| {
                            if let AuthAction::Read {
                                table_name,
                                column_name: "definition",
                            } = context.action
                                && context.accessor.is_none()
                            {
                                let index = match table_name {
                                    "gl_workspace_schemas" => Some(0),
                                    "gl_schema_history" => Some(1),
                                    _ => None,
                                };
                                if let Some(index) = index {
                                    observed[index].fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Authorization::Allow
                        }))
                        .map_err(sqlite_error)?;
                    }
                    let edits = (0..edit_count).map(|i| json!({"dataset":datasets[i % dataset_count],"feature_id":format!("feature-{i:03}"),"feature":{"type":"Feature","id":format!("feature-{i:03}"),"properties":{"label":format!("value-{i}")},"geometry":null}})).collect::<Vec<_>>();
                    let before = t.0.query_count;
                    let result = crate::workspace::save(
                        &mut t,
                        "alice",
                        crate::decode(
                            json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"edits":edits}),
                        )?,
                    )?;
                    let actual = reads.each_ref().map(|count| count.load(Ordering::Relaxed));
                    eprintln!(
                        "schema reads: source={source}, datasets={dataset_count}, edits={edit_count}, workspace={}, history={}, all_queries={}",
                        actual[0],
                        actual[1],
                        t.0.query_count - before
                    );
                    assert_eq!(
                        actual,
                        [
                            1,
                            if source == "workspace" {
                                0
                            } else {
                                dataset_count
                            }
                        ]
                    );
                    assert_eq!(
                        result["changes"],
                        edit_count
                            + if source == "workspace" {
                                dataset_count
                            } else {
                                0
                            }
                    );
                    // Dropping the transaction keeps every sample on the same fresh workspace.
                }
            }
        }
        Ok(())
    }
}
