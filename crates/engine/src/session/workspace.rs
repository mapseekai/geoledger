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
        // Stale SQLite statistics must not turn a single-key lookup into a history scan.
        let history = match self.0.backend {
            Backend::Sqlite(_) => "gl_history h INDEXED BY sqlite_autoindex_gl_history_1",
            Backend::Postgis(_) => "gl_history h",
        };
        self.query_one(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4 AND resolved_head IS NOT NULL),h.properties,h.geometry_json FROM (SELECT 1) seed LEFT JOIN (SELECT properties,geometry_json FROM {history} WHERE h.project=$1 AND h.dataset=$3 AND h.feature_id=$4 AND h.valid_from<=$5 AND (h.valid_to IS NULL OR h.valid_to>$5) ORDER BY h.valid_from DESC LIMIT 1) h ON true", self.workspace_key_table()), &[&project, &workspace, &dataset, &key, &base])
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
        // Aggregate in primary-key dataset order before joining names. Grouping by
        // names too makes SQLite sort full feature payloads into a temporary B-tree.
        self.query("WITH counts AS MATERIALIZED (SELECT c.dataset,SUM(CASE WHEN b.properties IS NULL AND c.properties IS NOT NULL THEN 1 ELSE 0 END) AS added,SUM(CASE WHEN b.properties IS NOT NULL AND c.properties IS NULL THEN 1 ELSE 0 END) AS deleted,SUM(CASE WHEN b.properties IS NOT NULL AND c.properties IS NOT NULL THEN 1 ELSE 0 END) AS modified FROM gl_workspace_changes c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) WHERE c.project=$1 AND c.workspace=$2 AND (c.properties IS NOT NULL OR b.properties IS NOT NULL) GROUP BY c.dataset) SELECT c.dataset,d.name,c.added,c.deleted,c.modified FROM counts c JOIN gl_datasets d ON d.project=$1 AND d.id=c.dataset ORDER BY c.dataset", &[&project,&workspace,&base])
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
        self.query("WITH batch AS MATERIALIZED (SELECT project,dataset,feature_id,properties,geometry_json,resolved_head,resolution_stale FROM gl_workspace_changes WHERE project=$1 AND workspace=$2 AND (dataset,feature_id)>($4,$5) ORDER BY dataset,feature_id LIMIT $6) SELECT c.dataset,c.feature_id,c.properties,c.geometry_json,b.properties,b.geometry_json FROM batch c LEFT JOIN gl_history b ON b.project=c.project AND b.dataset=c.dataset AND b.feature_id=c.feature_id AND b.valid_from<=$3 AND (b.valid_to IS NULL OR b.valid_to>$3) ORDER BY c.dataset,c.feature_id", &[&project, &workspace, &base, &after_dataset, &after_key, &limit])
    }

    fn diff_has_after(
        &mut self,
        project: &str,
        workspace: &str,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<bool> {
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
