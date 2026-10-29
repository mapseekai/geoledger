use super::*;

impl HistoryRepository for SqlTransaction {
    fn commit_summary(&mut self, project: &str, revision: i64) -> Result<Vec<Row>> {
        self.query("WITH counts AS MATERIALIZED (SELECT dataset,SUM(CASE WHEN NOT had_before AND has_after THEN 1 ELSE 0 END) AS added,SUM(CASE WHEN had_before AND NOT has_after THEN 1 ELSE 0 END) AS deleted,SUM(CASE WHEN had_before AND has_after THEN 1 ELSE 0 END) AS modified FROM gl_commit_changes WHERE project=$1 AND revision=$2 GROUP BY dataset) SELECT c.dataset,d.name,c.added,c.deleted,c.modified FROM counts c JOIN gl_datasets d ON d.project=$1 AND d.id=c.dataset ORDER BY c.dataset", &[&project,&revision])
    }

    fn history_page(&mut self, project: &str, after: i64, limit: i64) -> Result<Vec<Row>> {
        self.query("SELECT c.revision,c.subject,c.message,c.created_at,c.workspace,w.base_revision FROM gl_commits c JOIN gl_workspaces w ON w.project=c.project AND w.id=c.workspace WHERE c.project=$1 AND c.revision>$2 ORDER BY c.revision LIMIT $3", &[&project, &after, &limit])
    }

    fn history_has_after(&mut self, project: &str, after: i64) -> Result<bool> {
        self.query_one("SELECT EXISTS(SELECT 1 FROM gl_commits c JOIN gl_workspaces w ON w.project=c.project AND w.id=c.workspace WHERE c.project=$1 AND c.revision>$2)", &[&project, &after])?.get(0usize)
    }

    fn commit_exists(&mut self, project: &str, revision: i64) -> Result<Option<Row>> {
        self.query_opt(
            "SELECT 1 FROM gl_commits WHERE project=$1 AND revision=$2",
            &[&project, &revision],
        )
    }

    fn commit_page(
        &mut self,
        project: &str,
        revision: i64,
        after_dataset: &str,
        after_key: &str,
        limit: i64,
    ) -> Result<Vec<Row>> {
        let keys = self.query("SELECT dataset,feature_id FROM gl_commit_changes WHERE project=$1 AND revision=$2 AND (dataset,feature_id)>($3,$4) ORDER BY dataset,feature_id LIMIT $5", &[&project, &revision, &after_dataset, &after_key, &limit])?;
        let mut rows = Vec::with_capacity(keys.len());
        for key in keys {
            let dataset: String = key.get(0usize)?;
            let feature_id: String = key.get(1usize)?;
            let before =
                FeatureRepository::feature_at(self, project, &dataset, &feature_id, revision - 1)?;
            let after =
                FeatureRepository::feature_at(self, project, &dataset, &feature_id, revision)?;
            rows.push(Row::new(vec![
                Cell::Text(dataset),
                Cell::Text(feature_id),
                before.as_ref().map_or(Cell::Null, |r| r.0[0].clone()),
                before.as_ref().map_or(Cell::Null, |r| r.0[1].clone()),
                after.as_ref().map_or(Cell::Null, |r| r.0[0].clone()),
                after.as_ref().map_or(Cell::Null, |r| r.0[1].clone()),
            ]));
        }
        Ok(rows)
    }

    fn commit_has_after(
        &mut self,
        project: &str,
        revision: i64,
        after_dataset: &str,
        after_key: &str,
    ) -> Result<bool> {
        self.query_one("SELECT EXISTS(SELECT 1 FROM gl_commit_changes WHERE project=$1 AND revision=$2 AND (dataset,feature_id)>($3,$4))", &[&project, &revision, &after_dataset, &after_key])?.get(0usize)
    }

    fn count_commit_changes(&mut self, project: &str, revision: i64) -> Result<Row> {
        self.query_one(
            "SELECT count(*) FROM gl_commit_changes WHERE project=$1 AND revision=$2",
            &[&project, &revision],
        )
    }
}
