use super::*;

impl PublicationRepository for SqlTransaction {
    fn stage_schema_merge(
        &mut self,
        project: &str,
        workspace: &str,
        current: i64,
    ) -> Result<usize> {
        self.materialize_schema_merge(project, workspace, current)
    }
    fn restore_workspace_schemas(
        &mut self,
        project: &str,
        workspace: &str,
        revision: i64,
        current: i64,
    ) -> Result<()> {
        self.restore_schema_definitions(project, workspace, revision, current)
    }

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
        let mut rows = self.query("WITH keys AS MATERIALIZED (SELECT dataset,feature_id FROM gl_conflict_keys WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($3,$4) ORDER BY dataset,feature_id LIMIT $5) SELECT c.dataset,c.feature_id,c.properties,c.geometry_json,c.resolved_head,c.resolution_stale FROM keys k JOIN gl_workspace_changes c ON c.project=$1 AND c.workspace=$2 AND c.dataset=k.dataset AND c.feature_id=k.feature_id ORDER BY c.dataset,c.feature_id", &[&project,&workspace,&after_dataset,&after_key,&limit])?;
        self.fill_merge_versions(project, base, head, &mut rows)?;
        Ok(rows)
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
        let mut rows = self.query("SELECT dataset,feature_id,properties,geometry_json,resolved_head,resolution_stale FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($3,$4) ORDER BY dataset,feature_id LIMIT 32", &[&project,&workspace,&after_dataset,&after_key])?;
        self.fill_merge_versions(project, base, current, &mut rows)?;
        Ok(rows)
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
        self.execute("INSERT INTO gl_commit_changes SELECT $1,$2,dataset,feature_id,before_value IS NOT NULL,after_value IS NOT NULL FROM gl_merge_stage", &[&project, &revision])
    }

    fn close_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        if matches!(self.0.backend, Backend::Postgis(_)) {
            // Pooled sessions retain the old batch's statistics after commit
            // clears the temp table. Refresh its cardinality for this batch.
            self.batch_execute("ANALYZE gl_merge_stage")?;
        }
        let prepared = self.publish_schema_definitions(project, revision)?;
        self.sync_sources(project, &prepared)?;
        if matches!(self.0.backend, Backend::Postgis(_)) {
            return self.close_postgis_history(project, revision);
        }
        let mut after = (String::new(), String::new());
        loop {
            let rows = self.query("SELECT dataset,feature_id,before_value FROM gl_merge_stage WHERE before_value IS NOT NULL AND (dataset,feature_id)>($1,$2) ORDER BY dataset,feature_id LIMIT 32", &[&after.0,&after.1])?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let dataset: String = row.get(0usize)?;
                let key: String = row.get(1usize)?;
                let before: String = row.get(2usize)?;
                let start = self.current_start(project, &dataset, &key)?;
                let columns = self.geometry_columns();
                self.execute(&format!("INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties,geometry_json,{columns}) VALUES($1,$2,$3,$4,$5,gl_json_field($6,'properties'),gl_json_field($6,'geometry'),{values})",values=self.geometry_values("gl_json_field($6,'geometry')")), &[&project,&dataset,&key,&start,&revision,&before])?;
                after = (dataset, key);
            }
        }
        Ok(())
    }

    fn append_history(&mut self, project: &str, revision: i64) -> Result<()> {
        self.flush_stage()?;
        if matches!(self.0.backend, Backend::Postgis(_)) {
            self.execute("DELETE FROM gl_source_versions v USING gl_merge_stage m,gl_datasets d WHERE v.project=$1 AND v.dataset=m.dataset AND v.feature_id=m.feature_id AND m.after_value IS NULL AND d.project=$1 AND d.id=m.dataset AND d.postgis_source IS NOT NULL", &[&project])?;
            self.execute("INSERT INTO gl_source_versions(project,dataset,feature_id,valid_from) SELECT $1,m.dataset,m.feature_id,$2 FROM gl_merge_stage m JOIN gl_datasets d ON d.project=$1 AND d.id=m.dataset WHERE d.postgis_source IS NOT NULL AND m.after_value IS NOT NULL ON CONFLICT(project,dataset,feature_id) DO UPDATE SET valid_from=excluded.valid_from", &[&project,&revision])?;
        }
        let mut after = String::new();
        loop {
            let rows = self.query("SELECT DISTINCT dataset FROM gl_merge_stage WHERE dataset>$1 ORDER BY dataset LIMIT 32", &[&after])?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let dataset: String = row.get(0usize)?;
                self.write_dataset_table(project, &dataset, revision)?;
                after = dataset;
            }
        }
        Ok(())
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
        let mut after = (String::new(), String::new());
        loop {
            let rows = self.query("SELECT dataset,feature_id FROM gl_commit_changes WHERE project=$1 AND revision=$2 AND (dataset,feature_id)>($3,$4) ORDER BY dataset,feature_id LIMIT 32", &[&project,&revision,&after.0,&after.1])?;
            if rows.is_empty() {
                break;
            }
            let mut deltas = Vec::with_capacity(rows.len());
            for row in rows {
                let dataset: String = row.get(0usize)?;
                let key: String = row.get(1usize)?;
                let before =
                    FeatureRepository::feature_at(self, project, &dataset, &key, revision - 1)?;
                let (properties, geometry) = if let Some(before) = before {
                    (before.get(0usize)?, before.get(1usize)?)
                } else {
                    (None, None)
                };
                deltas.push(crate::repository::DraftDelta {
                    dataset: dataset.clone(),
                    key: key.clone(),
                    properties,
                    geometry,
                });
                after = (dataset, key);
            }
            self.put_deltas(project, workspace, &deltas)?;
        }
        Ok(())
    }
}

impl SqlTransaction {
    fn fill_merge_versions(
        &mut self,
        project: &str,
        base: i64,
        head: i64,
        rows: &mut [Row],
    ) -> Result<()> {
        let mut remaining = rows;
        while let Some(first) = remaining.first() {
            let dataset: String = first.get(0usize)?;
            let mut count = 1;
            while count < remaining.len() && remaining[count].get::<_, String>(0usize)? == dataset {
                count += 1;
            }
            let (rows, rest) = remaining.split_at_mut(count);
            remaining = rest;
            if matches!(self.0.backend, Backend::Postgis(_))
                && let Some(metadata) = self.dataset_exists(project, &dataset)?
                && metadata.get::<_, Option<String>>(3usize)?.is_some()
            {
                self.fill_source_merge_versions(project, &dataset, &metadata, base, head, rows)?;
                continue;
            }
            for row in rows {
                let dataset: String = row.get(0usize)?;
                let key: String = row.get(1usize)?;
                let original = super::query::version_cells(FeatureRepository::feature_at(
                    self, project, &dataset, &key, base,
                )?);
                let current = if base == head {
                    original.clone()
                } else {
                    super::query::version_cells(FeatureRepository::feature_at(
                        self, project, &dataset, &key, head,
                    )?)
                };
                row.0.extend(original);
                row.0.extend(current);
            }
        }
        Ok(())
    }

    fn fill_source_merge_versions(
        &mut self,
        project: &str,
        dataset: &str,
        metadata: &Row,
        base: i64,
        head: i64,
        rows: &mut [Row],
    ) -> Result<()> {
        let source: super::tables::Source =
            crate::codec::stored(&metadata.get::<_, String>(3usize)?)?;
        self.ensure_source_clean(project, dataset, &source)?;
        let table = super::tables::relation(&source.table);
        let id = super::tables::quote(&source.table.id_column);
        let geom = super::tables::quote(&source.table.geometry_column);
        let start: i64 = metadata.get(6usize)?;
        let keys = rows
            .iter()
            .map(|r| r.get::<_, String>(1usize))
            .collect::<Result<Vec<_>>>()?;
        let keys = serde_json::to_string(&keys).map_err(Error::stored_json)?;
        let limit = i64::try_from(self.response_limit()).unwrap_or(i64::MAX);
        let sql = format!(
            "WITH versions AS MATERIALIZED (
                SELECT k.ordinal,
                    CASE WHEN h.feature_id IS NOT NULL THEN h.properties ELSE c.properties END AS properties,
                    CASE WHEN h.feature_id IS NOT NULL THEN h.geometry_json ELSE c.geometry_json END AS geometry_json
                FROM jsonb_array_elements_text($4::text::jsonb) WITH ORDINALITY k(key,ordinal)
                LEFT JOIN LATERAL (SELECT feature_id,properties,geometry_json FROM gl_history
                    WHERE project=$1 AND dataset=$2 AND feature_id=k.key AND valid_from<=$3 AND valid_to>$3
                    ORDER BY valid_from DESC LIMIT 1) h ON true
                LEFT JOIN LATERAL (SELECT (to_jsonb(t)-$5::text-$6::text)::text AS properties,ST_AsGeoJSON(t.{geom},17,0) AS geometry_json
                    FROM {table} t LEFT JOIN gl_source_versions v ON v.project=$1 AND v.dataset=$2 AND v.feature_id=t.{id}::text COLLATE \"C\"
                    WHERE h.feature_id IS NULL AND t.{id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($5::text,k.key))).{id}
                    AND coalesce(v.valid_from,$7::bigint)<=$3) c ON true
                WHERE k.ordinal>$9
            ), bounded AS (
                SELECT *,sum(8::bigint+coalesce(octet_length(convert_to(properties,'UTF8')),0)+coalesce(octet_length(convert_to(geometry_json,'UTF8')),0)) OVER (ORDER BY ordinal) AS bytes,
                    row_number() OVER (ORDER BY ordinal) AS position FROM versions
            ) SELECT properties,geometry_json FROM bounded WHERE bytes<=$8::bigint OR position=1 ORDER BY ordinal"
        );
        for revision in [base, head] {
            let mut after = 0;
            while after < rows.len() {
                // Two column length words plus payload match the existing PostgreSQL result budget.
                let versions = self.query(
                    &sql,
                    &[
                        &project,
                        &dataset,
                        &revision,
                        &keys,
                        &source.table.id_column,
                        &source.table.geometry_column,
                        &start,
                        &limit,
                        &(after as i64),
                    ],
                )?;
                if versions.is_empty() {
                    return Err(Error::new(500, "missing merge versions"));
                }
                let count = versions.len();
                for (row, version) in rows[after..].iter_mut().zip(versions) {
                    if base == head {
                        row.0.extend(version.0.clone());
                    }
                    row.0.extend(version.0);
                }
                after += count;
            }
            if base == head {
                break;
            }
        }
        Ok(())
    }

    fn close_postgis_history(&mut self, project: &str, revision: i64) -> Result<()> {
        let mut after = String::new();
        loop {
            let rows = self.query("SELECT DISTINCT dataset FROM gl_merge_stage WHERE before_value IS NOT NULL AND dataset>$1 ORDER BY dataset LIMIT 32", &[&after])?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let dataset: String = row.get(0usize)?;
                let metadata = self
                    .dataset_exists(project, &dataset)?
                    .ok_or_else(|| Error::new(500, "missing dataset"))?;
                let join = if let Some(table) = metadata.get::<_, Option<String>>(5usize)? {
                    super::dataset::check_table_name(&table)?;
                    format!("JOIN {table} c ON c.feature_id=m.feature_id")
                } else {
                    "LEFT JOIN gl_source_versions c ON c.project=$1 AND c.dataset=$2 AND c.feature_id=m.feature_id".to_owned()
                };
                let start: i64 = metadata.get(6usize)?;
                let complete: bool = self.query_one(&format!("WITH archived AS (INSERT INTO gl_history(project,dataset,feature_id,valid_from,valid_to,properties,geometry_json,geom)
                    SELECT $1,$2,m.feature_id,coalesce(c.valid_from,$4::bigint),$3,gl_json_field(m.before_value,'properties'),gl_json_field(m.before_value,'geometry'),{geometry}
                    FROM gl_merge_stage m {join} WHERE m.dataset=$2 AND m.before_value IS NOT NULL RETURNING 1)
                    SELECT (SELECT count(*) FROM archived)=(SELECT count(*) FROM gl_merge_stage WHERE dataset=$2 AND before_value IS NOT NULL)", geometry=self.geometry_values("gl_json_field(m.before_value,'geometry')")), &[&project,&dataset,&revision,&start])?.get(0usize)?;
                if !complete {
                    return Err(Error::new(500, "missing current feature"));
                }
                after = dataset;
            }
        }
        Ok(())
    }

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
