use super::*;

impl PublicationRepository for SqlTransaction {
    fn cached_conflict_count(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        version: i64,
        head: i64,
    ) -> Result<Option<i64>> {
        self.query_opt("SELECT total FROM gl_conflict_cache WHERE project=$1 AND workspace=$2 AND base_revision=$3 AND workspace_version=$4 AND head=$5 AND complete", &[&project,&workspace,&base,&version,&head])?.map(|r|r.get(0usize)).transpose()
    }

    fn reset_conflict_cache(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        version: i64,
        head: i64,
    ) -> Result<()> {
        self.execute(
            "DELETE FROM gl_conflict_cache WHERE project=$1 AND workspace=$2",
            &[&project, &workspace],
        )?;
        self.execute("INSERT INTO gl_conflict_cache(project,workspace,base_revision,workspace_version,head) VALUES($1,$2,$3,$4,$5)", &[&project,&workspace,&base,&version,&head])
    }

    fn cache_conflict_keys(
        &mut self,
        project: &str,
        workspace: &str,
        keys: &[(String, String)],
    ) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut params: Vec<&dyn Parameter> = vec![&project, &workspace];
        let mut tuples = Vec::with_capacity(keys.len());
        for (i, (dataset, key)) in keys.iter().enumerate() {
            tuples.push(format!("($1,$2,${},${})", i * 2 + 3, i * 2 + 4));
            params.extend([dataset as &dyn Parameter, key]);
        }
        self.execute(
            &format!("INSERT INTO gl_conflict_keys VALUES {}", tuples.join(",")),
            &params,
        )
    }

    fn complete_conflict_cache(
        &mut self,
        project: &str,
        workspace: &str,
        total: i64,
    ) -> Result<()> {
        self.execute(
            "UPDATE gl_conflict_cache SET total=$3,complete=true WHERE project=$1 AND workspace=$2",
            &[&project, &workspace, &total],
        )
    }

    fn cached_conflict_page(
        &mut self,
        query: &crate::repository::ConflictQuery<'_>,
    ) -> Result<Vec<Row>> {
        let crate::repository::ConflictQuery {
            project,
            workspace,
            base,
            head,
            after_dataset,
            after_key,
            limit,
        } = *query;
        self.query("WITH keys AS MATERIALIZED (SELECT dataset,feature_id FROM gl_conflict_keys WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($5,$6) ORDER BY dataset,feature_id LIMIT $7), batch AS MATERIALIZED (SELECT c.* FROM keys k JOIN gl_workspace_changes c ON c.project=$1 AND c.workspace=$2 AND c.dataset=k.dataset AND c.feature_id=k.feature_id) SELECT c.dataset,c.feature_id,c.properties,c.geometry_json,c.resolved_head,c.resolution_stale,b.properties,b.geometry_json,o.properties,o.geometry_json FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) LEFT JOIN gl_history o ON o.project=c.project AND o.dataset=c.dataset AND o.feature_id=c.feature_id AND o.valid_from<=$4 AND (o.valid_to IS NULL OR o.valid_to>$4) ORDER BY c.dataset,c.feature_id", &[&project,&workspace,&base,&head,&after_dataset,&after_key,&limit])
    }

    fn begin_merge(&mut self) -> Result<()> {
        self.1.stage.clear();
        self.1.stage_bytes = 0;
        // Pooled PostgreSQL sessions reuse the staging relation and its prepared plans.
        // Transactional deletion keeps repeated merges and rollback isolated.
        self.batch_execute(match self.0.backend {
            Backend::Postgis(_) => "CREATE TEMP TABLE IF NOT EXISTS gl_merge_stage(dataset text, feature_id text, before_value text, after_value text, PRIMARY KEY(dataset,feature_id)) ON COMMIT DELETE ROWS; DELETE FROM gl_merge_stage",
            Backend::Sqlite(_) => "DROP TABLE IF EXISTS gl_merge_stage; CREATE TEMP TABLE gl_merge_stage(dataset text, feature_id text, before_value text, after_value text, PRIMARY KEY(dataset,feature_id))",
        })
    }

    fn merge_page(
        &mut self,
        project: &str,
        workspace: &str,
        base: i64,
        current: i64,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<Vec<Row>> {
        self.query("WITH batch AS MATERIALIZED (SELECT project,dataset,feature_id,properties,geometry_json,resolved_head,resolution_stale FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($5,$6) ORDER BY dataset,feature_id LIMIT 32) SELECT c.dataset,c.feature_id,c.properties,c.geometry_json,c.resolved_head,c.resolution_stale,b.properties,b.geometry_json,o.properties,o.geometry_json FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) LEFT JOIN gl_history o ON o.project=c.project AND o.dataset=c.dataset AND o.feature_id=c.feature_id AND o.valid_from<=$4 AND (o.valid_to IS NULL OR o.valid_to>$4) ORDER BY c.dataset,c.feature_id", &[&project, &workspace, &base, &current, &after_dataset, &after_key])
    }

    fn stage_merge(
        &mut self,
        dataset: &str,
        key: &str,
        before: &Option<String>,
        after: &Option<String>,
    ) -> Result<()> {
        let bytes = dataset
            .len()
            .saturating_add(key.len())
            .saturating_add(before.as_ref().map_or(0, String::len))
            .saturating_add(after.as_ref().map_or(0, String::len));
        if self.1.stage_bytes.saturating_add(bytes) > 1024 * 1024 {
            self.flush_stage()?;
        }
        self.1.stage.push((
            dataset.to_owned(),
            key.to_owned(),
            before.clone(),
            after.clone(),
        ));
        self.1.stage_bytes = self.1.stage_bytes.saturating_add(bytes);
        if self.1.stage.len() >= 32 || self.1.stage_bytes >= 1024 * 1024 {
            self.flush_stage()?;
        }
        Ok(())
    }

    fn publication_receipt(
        &mut self,
        project: &str,
        subject: &str,
        request_id: &str,
    ) -> Result<Option<Row>> {
        self.query_opt("SELECT payload,result FROM gl_idempotency WHERE project=$1 AND subject=$2 AND request_id=$3", &[&project, &subject, &request_id])
    }

    fn append_commit(
        &mut self,
        project: &str,
        revision: i64,
        workspace: &str,
        subject: &str,
        message: &str,
    ) -> Result<()> {
        self.execute("INSERT INTO gl_commits(project,revision,workspace,subject,message) VALUES($1,$2,$3,$4,$5)", &[&project, &revision, &workspace, &subject, &message])
    }

    fn append_changes(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        self.execute("INSERT INTO gl_commit_changes SELECT $1,$2,dataset,feature_id,NULLIF(before_value,'null'),after_value FROM gl_merge_stage", &[&project, &revision])
    }

    fn close_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        if matches!(self.0.backend, Backend::Postgis(_)) {
            // Pooled sessions retain the old batch's statistics after commit
            // clears the temp table. Refresh its cardinality for this batch.
            self.batch_execute("ANALYZE gl_merge_stage")?;
        }
        self.sync_sources(project)?;
        // Drive both backends from changed keys, even when a new project's
        // history is underestimated by statistics dominated by another project.
        let sql = match self.0.backend {
            Backend::Sqlite(_) => {
                "UPDATE gl_history SET valid_to=$2 WHERE rowid IN (SELECT h.rowid FROM gl_merge_stage m CROSS JOIN gl_history h INDEXED BY history_live ON h.project=$1 AND h.dataset=m.dataset AND h.feature_id=m.feature_id AND h.valid_to IS NULL)"
            }
            Backend::Postgis(_) => {
                // LIMIT keeps each live-key lookup correlated with the staged
                // row. The TID is consumed in this statement while the project
                // publication lock protects the history rows.
                "UPDATE gl_history AS h SET valid_to=$2 FROM gl_merge_stage m CROSS JOIN LATERAL (SELECT ctid FROM gl_history WHERE project=$1 AND dataset=m.dataset AND feature_id=m.feature_id AND valid_to IS NULL LIMIT 1) live WHERE h.ctid=live.ctid"
            }
        };
        self.execute(sql, &[&project, &revision])
    }

    fn append_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        self.execute(&format!("INSERT INTO gl_history(project,dataset,feature_id,valid_from,properties,geometry_json,{columns}) SELECT $1,dataset,feature_id,$2,gl_json_field(after_value,'properties'),gl_json_field(after_value,'geometry'),{values} FROM gl_merge_stage", columns=self.geometry_columns(), values=self.geometry_values("gl_json_field(after_value,'geometry')")), &[&project, &revision])
    }

    fn advance_head(&mut self, project: &str, revision: i64) -> Result<()> {
        self.execute(
            "UPDATE gl_projects SET head=$2 WHERE id=$1",
            &[&project, &revision],
        )
    }

    fn save_receipt(
        &mut self,
        project: &str,
        subject: &str,
        request_id: &str,
        payload: &str,
        result: &str,
    ) -> Result<()> {
        self.execute(
            "INSERT INTO gl_idempotency VALUES($1,$2,$3,$4,$5)",
            &[&project, &subject, &request_id, &payload, &result],
        )
    }

    fn stage_resolution(&mut self, dataset: &str, key: &str, after: &Option<String>) -> Result<()> {
        self.execute(
            "INSERT INTO gl_merge_stage(dataset,feature_id,after_value) VALUES($1,$2,$3)",
            &[&dataset, &key, &after],
        )
    }

    fn replace_deltas_with_merge(&mut self, project: &str, workspace: &str) -> Result<()> {
        self.flush_stage()?;
        self.execute(&format!("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geometry_json,{columns}) SELECT $1,$2,dataset,feature_id,gl_json_field(after_value,'properties'),gl_json_field(after_value,'geometry'),{values} FROM gl_merge_stage", columns=self.geometry_columns(), values=self.geometry_values("gl_json_field(after_value,'geometry')")), &[&project, &workspace])
    }

    fn restore_deltas(&mut self, project: &str, revision: i64, workspace: &str) -> Result<()> {
        self.execute(&format!("INSERT INTO gl_workspace_changes(project,workspace,dataset,feature_id,properties,geometry_json,{columns}) SELECT project,$3,dataset,feature_id,gl_json_field(before_value,'properties'),gl_json_field(before_value,'geometry'),{values} FROM gl_commit_changes WHERE project=$1 AND revision=$2", columns=self.geometry_columns(), values=self.geometry_values("gl_json_field(before_value,'geometry')")), &[&project, &revision, &workspace])
    }
}

impl SqlTransaction {
    pub(super) fn flush_stage(&mut self) -> Result<()> {
        if self.1.stage.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.1.stage);
        self.1.stage_bytes = 0;
        let tuples: Vec<_> = (0..rows.len())
            .map(|i| {
                format!(
                    "(${},${},${},${})",
                    i * 4 + 1,
                    i * 4 + 2,
                    i * 4 + 3,
                    i * 4 + 4
                )
            })
            .collect();
        let mut params: Vec<&dyn Parameter> = Vec::with_capacity(rows.len() * 4);
        for (dataset, key, before, after) in &rows {
            params.extend([dataset as &dyn Parameter, key, before, after]);
        }
        self.execute(
            &format!("INSERT INTO gl_merge_stage VALUES {}", tuples.join(",")),
            &params,
        )
    }
}
