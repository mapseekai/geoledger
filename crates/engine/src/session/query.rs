use super::*;

impl FeatureRepository for SqlTransaction {
    fn feature_page(&mut self, r: &crate::repository::FeatureQuery) -> Result<Vec<Row>> {
        self.feature_rows(r, true)
    }

    fn feature_exists(&mut self, r: &crate::repository::FeatureQuery) -> Result<bool> {
        let query = crate::repository::FeatureQuery {
            limit: 1,
            ..r.clone()
        };
        Ok(!self.feature_rows(&query, false)?.is_empty())
    }

    fn feature_at(
        &mut self,
        project: &str,
        dataset: &str,
        key: &str,
        revision: i64,
    ) -> Result<Option<Row>> {
        self.query_opt("SELECT properties,geometry_json FROM gl_history WHERE project=$1 AND dataset=$2 AND feature_id=$3 AND valid_from<=$4 AND (valid_to IS NULL OR valid_to>$4) ORDER BY valid_from DESC LIMIT 1", &[&project, &dataset, &key, &revision])
    }
}

impl SqlTransaction {
    fn feature_rows(
        &mut self,
        r: &crate::repository::FeatureQuery,
        include_payload: bool,
    ) -> Result<Vec<Row>> {
        let bbox = r.bbox.unwrap_or([-180., -90., 180., 90.]);
        if r.bbox.is_some()
            && matches!(self.0.backend, Backend::Postgis(_))
            && !self.1.custom_spatial_plan
        {
            // Bbox selectivity varies by orders of magnitude. Keep parameter-aware
            // plans for this transaction without disabling prepared statements globally.
            self.batch_execute("SET LOCAL plan_cache_mode=force_custom_plan")?;
            self.1.custom_spatial_plan = true;
        }
        let latest = self.1.heads.get(&r.project) == Some(&r.revision);
        let visible = if latest {
            "h.valid_to IS NULL"
        } else {
            "(h.valid_to IS NULL OR h.valid_to>$3)"
        };
        // Bound both UNION branches before geometry/JSON expansion and final
        // sorting. A top-level LIMIT alone let PostgreSQL scan and serialize
        // the entire remaining dataset for each 32-row page.
        let key_filter = if r.feature_id.is_some() {
            "feature_id=$6"
        } else {
            "CAST($6 AS text) IS NULL"
        };
        let sparse_history = self.sparse_spatial_candidates("history", r)?;
        let sparse_draft = self.sparse_spatial_candidates("workspace_changes", r)?;
        let spatial_filter = self.spatial_filter("h", "history", r.bbox.is_some(), sparse_history);
        let draft_spatial_filter =
            self.spatial_filter("", "workspace_changes", r.bbox.is_some(), sparse_draft);
        let (spatial_lookup, draft_lookup) = if matches!(self.0.backend, Backend::Sqlite(_)) {
            (
                if latest {
                    " INDEXED BY history_live"
                } else {
                    " INDEXED BY sqlite_autoindex_gl_history_1"
                },
                " INDEXED BY sqlite_autoindex_gl_workspace_changes_1",
            )
        } else {
            ("", "")
        };
        let base_payload = if include_payload {
            ",h.properties,h.geometry_json"
        } else {
            ""
        };
        let draft_payload = if include_payload {
            ",properties,geometry_json"
        } else {
            ""
        };
        let sql = format!("WITH base AS MATERIALIZED (
            SELECT h.feature_id{base_payload} FROM gl_history h{spatial_lookup}
            WHERE h.project=$1 AND h.dataset=$2
              AND h.valid_from<=$3 AND {visible}
              AND h.properties IS NOT NULL AND h.feature_id>$5 AND {key_filter}
              AND ({spatial_filter})
              AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c
                WHERE c.project=h.project AND c.dataset=h.dataset AND c.feature_id=h.feature_id AND c.workspace=$4)
            ORDER BY h.feature_id LIMIT $12
          ), draft AS MATERIALIZED (
            SELECT feature_id{draft_payload} FROM gl_workspace_changes{draft_lookup}
            WHERE project=$1 AND dataset=$2 AND workspace=$4
              AND properties IS NOT NULL AND feature_id>$5 AND {key_filter}
              AND ({draft_spatial_filter})
            ORDER BY feature_id LIMIT $12
          ), page AS MATERIALIZED (
            SELECT * FROM base UNION ALL SELECT * FROM draft ORDER BY feature_id LIMIT $12
          ) SELECT * FROM page ORDER BY feature_id");
        self.query(
            &sql,
            &[
                &r.project,
                &r.dataset,
                &r.revision,
                &r.workspace,
                &r.after,
                &r.feature_id,
                &r.bbox.is_some(),
                &bbox[0],
                &bbox[1],
                &bbox[2],
                &bbox[3],
                &r.limit,
            ],
        )
    }

    pub(super) fn sparse_spatial_candidates(
        &mut self,
        table: &str,
        r: &crate::repository::FeatureQuery,
    ) -> Result<bool> {
        let Some(bbox) = r.bbox else { return Ok(false) };
        if r.feature_id.is_some() || !matches!(self.0.backend, Backend::Sqlite(_)) {
            return Ok(false);
        }
        // Small Rtree results avoid even the bounded feature-key probe below.
        let cap = r.limit.saturating_mul(4);
        let count: i64 = self.query_one(&format!("SELECT count(*) FROM (
            SELECT pkid FROM idx_gl_{table}_geom WHERE xmin<=$3 AND xmax>=$1 AND ymin<=$4 AND ymax>=$2
            UNION ALL SELECT pkid FROM idx_gl_{table}_geom_z WHERE xmin<=$3 AND xmax>=$1 AND ymin<=$4 AND ymax>=$2 LIMIT $5
        )"), &[&bbox[0], &bbox[1], &bbox[2], &bbox[3], &cap])?.get(0usize)?;
        if count < cap {
            return Ok(true);
        }
        // Global Rtree density says nothing about this dataset or cursor. Use
        // key order when a bounded prefix fills the page or exhausts the remaining
        // keys, so terminal lookahead does not enumerate a dense global Rtree.
        let (scope, visible, visibility_columns) = if table == "history" {
            (
                "project=$1 AND dataset=$2 AND feature_id>$5",
                "valid_from<=$3 AND (valid_to IS NULL OR valid_to>$3) AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c INDEXED BY sqlite_autoindex_gl_workspace_changes_1 WHERE c.project=$1 AND c.workspace=$4 AND c.dataset=$2 AND c.feature_id=prefix.feature_id)",
                ",valid_from,valid_to",
            )
        } else {
            (
                "project=$1 AND workspace=$4 AND dataset=$2 AND feature_id>$5",
                "CAST($3 AS bigint) IS NOT NULL",
                "",
            )
        };
        let row = self.query_one(
            &format!("WITH prefix AS MATERIALIZED (SELECT feature_id,properties IS NOT NULL AS present,geom,geom_z{visibility_columns} FROM gl_{table} INDEXED BY sqlite_autoindex_gl_{table}_1 WHERE {scope} ORDER BY feature_id LIMIT $10) SELECT (SELECT count(*) FROM (SELECT 1 FROM prefix WHERE present AND ({visible}) AND (ST_Intersects(geom,BuildMbr($6,$7,$8,$9,0))=1 OR ST_Intersects(geom_z,BuildMbr($6,$7,$8,$9,0))=1) LIMIT $11)), (SELECT count(*) FROM prefix)"),
            &[&r.project, &r.dataset, &r.revision, &r.workspace, &r.after, &bbox[0], &bbox[1], &bbox[2], &bbox[3], &cap, &r.limit],
        )?;
        let matches: i64 = row.get(0usize)?;
        let remaining: i64 = row.get(1usize)?;
        Ok(matches < r.limit && remaining >= cap)
    }

    pub(crate) fn spatial_filter(
        &self,
        alias: &str,
        table: &str,
        enabled: bool,
        candidates: bool,
    ) -> String {
        // Cast unused parameters too: PostgreSQL prepares a single typed parameter vector.
        if !enabled {
            return "NOT $7 AND CAST($8 AS double precision) IS NOT NULL AND CAST($9 AS double precision) IS NOT NULL AND CAST($10 AS double precision) IS NOT NULL AND CAST($11 AS double precision) IS NOT NULL".into();
        }
        let a = if alias.is_empty() {
            String::new()
        } else {
            format!("{alias}.")
        };
        match self.0.backend {
            Backend::Postgis(_) => {
                format!("$7 AND ST_Intersects({a}geom,ST_MakeEnvelope($8,$9,$10,$11,0))")
            }
            Backend::Sqlite(_) if !candidates => format!(
                "$7 AND (ST_Intersects({a}geom,BuildMbr($8,$9,$10,$11,0))=1 OR ST_Intersects({a}geom_z,BuildMbr($8,$9,$10,$11,0))=1)"
            ),
            // +0 retains scalar rowid point lookup even after a stale one-row ANALYZE estimate.
            Backend::Sqlite(_) => format!(
                "$7 AND {a}feature_id IN (SELECT (SELECT feature_id FROM gl_{table} WHERE rowid=spatial_candidates.pkid+0) FROM (SELECT pkid FROM idx_gl_{table}_geom WHERE xmin<=$10 AND xmax>=$8 AND ymin<=$11 AND ymax>=$9 UNION ALL SELECT pkid FROM idx_gl_{table}_geom_z WHERE xmin<=$10 AND xmax>=$8 AND ymin<=$11 AND ymax>=$9) spatial_candidates) AND (ST_Intersects({a}geom,BuildMbr($8,$9,$10,$11,0))=1 OR ST_Intersects({a}geom_z,BuildMbr($8,$9,$10,$11,0))=1)"
            ),
        }
    }
}
