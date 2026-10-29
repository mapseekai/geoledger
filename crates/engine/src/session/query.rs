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
        if let Some(row) = self.query_opt("SELECT properties,geometry_json FROM gl_history WHERE project=$1 AND dataset=$2 AND feature_id=$3 AND valid_from<=$4 AND valid_to>$4 ORDER BY valid_from DESC LIMIT 1", &[&project, &dataset, &key, &revision])? {
            return Ok(Some(row));
        }
        self.current_at(project, dataset, key, revision)
    }
}

impl SqlTransaction {
    pub(super) fn current_at(
        &mut self,
        project: &str,
        dataset: &str,
        key: &str,
        revision: i64,
    ) -> Result<Option<Row>> {
        let Some(metadata) = self.dataset_exists(project, dataset)? else {
            return Ok(None);
        };
        if let Some(table) = metadata.get::<_, Option<String>>(5usize)? {
            super::dataset::check_table_name(&table)?;
            return self.query_opt(&format!("SELECT properties,geometry_json FROM {table} WHERE feature_id=$1 AND valid_from<=$2"), &[&key,&revision]);
        }
        let Some(source) = metadata.get::<_, Option<String>>(3usize)? else {
            return Ok(None);
        };
        let source: super::tables::Source = crate::codec::stored(&source)?;
        self.ensure_source_clean(project, dataset, &source)?;
        let table = super::tables::relation(&source.table);
        let id = super::tables::quote(&source.table.id_column);
        let geom = super::tables::quote(&source.table.geometry_column);
        let start: i64 = metadata.get(6usize)?;
        let properties = super::tables::source_properties("$5", "$6");
        self.query_opt(&format!("SELECT {properties}::text,ST_AsGeoJSON(t.{geom},17,0) FROM {table} t LEFT JOIN gl_source_versions v ON v.project=$1 AND v.dataset=$2 AND v.feature_id=t.{id}::text COLLATE \"C\" WHERE t.{id}=(jsonb_populate_record(NULL::{table},jsonb_build_object($5::text,$3::text))).{id} AND coalesce(v.valid_from,$7::bigint)<=$4"), &[&project,&dataset,&key,&revision,&source.table.id_column,&source.table.geometry_column,&start])
    }

    pub(super) fn current_start(&mut self, project: &str, dataset: &str, key: &str) -> Result<i64> {
        let metadata = self
            .dataset_exists(project, dataset)?
            .ok_or_else(|| Error::new(500, "missing dataset"))?;
        if let Some(table) = metadata.get::<_, Option<String>>(5usize)? {
            super::dataset::check_table_name(&table)?;
            return self
                .query_one(
                    &format!("SELECT valid_from FROM {table} WHERE feature_id=$1"),
                    &[&key],
                )?
                .get(0usize);
        }
        let start: i64 = metadata.get(6usize)?;
        self.query_one("SELECT coalesce((SELECT valid_from FROM gl_source_versions WHERE project=$1 AND dataset=$2 AND feature_id=$3),$4)", &[&project,&dataset,&key,&start])?.get(0usize)
    }
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
        let metadata = self
            .dataset_exists(&r.project, &r.dataset)?
            .ok_or_else(|| Error::new(404, "dataset not found"))?;
        let managed: Option<String> = metadata.get(5usize)?;
        let key_filter = if r.feature_id.is_some() {
            "h.feature_id=$6"
        } else {
            "CAST($6 AS text) IS NULL"
        };
        let sparse_draft = self.sparse_spatial_candidates("workspace_changes", r)?;
        let draft_filter =
            self.spatial_filter("", "workspace_changes", r.bbox.is_some(), sparse_draft);
        let draft_lookup = if matches!(self.0.backend, Backend::Sqlite(_)) {
            " INDEXED BY sqlite_autoindex_gl_workspace_changes_1"
        } else {
            ""
        };
        let payload = if include_payload {
            ",h.properties,h.geometry_json"
        } else {
            ""
        };
        let draft_payload = if include_payload {
            ",properties,geometry_json"
        } else {
            ""
        };
        let current = if let Some(table) = managed {
            super::dataset::check_table_name(&table)?;
            let sparse = self.sparse_spatial_candidates(&table[3..], r)?;
            let filter = self.spatial_filter("h", &table[3..], r.bbox.is_some(), sparse);
            let lookup = if matches!(self.0.backend, Backend::Sqlite(_)) {
                format!(" INDEXED BY sqlite_autoindex_{table}_1")
            } else {
                String::new()
            };
            format!(
                "SELECT h.feature_id{payload} FROM {table} h{lookup} WHERE h.valid_from<=$3 AND h.feature_id>$5 AND {key_filter} AND ({filter}) AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c WHERE c.project=$1 AND c.dataset=$2 AND c.workspace=$4 AND c.feature_id=h.feature_id) ORDER BY h.feature_id LIMIT $12"
            )
        } else {
            let source: String = metadata
                .get::<_, Option<String>>(3usize)?
                .ok_or_else(|| Error::new(500, "dataset has no storage table"))?;
            let source: super::tables::Source = crate::codec::stored(&source)?;
            self.ensure_source_clean(&r.project, &r.dataset, &source)?;
            let table = super::tables::relation(&source.table);
            let id = super::tables::quote(&source.table.id_column);
            let geom = super::tables::quote(&source.table.geometry_column);
            let key = format!("t.{id}::text COLLATE \"C\"");
            let source_payload = if include_payload {
                let properties = super::tables::source_properties(
                    &format!("'{}'", source.table.id_column.replace('\'', "''")),
                    &format!("'{}'", source.table.geometry_column.replace('\'', "''")),
                );
                format!(",{properties}::text,ST_AsGeoJSON(t.{geom},17,0)")
            } else {
                String::new()
            };
            let start: i64 = metadata.get(6usize)?;
            let filter = if r.bbox.is_some() {
                format!(
                    "$7 AND ST_Intersects(t.{geom},ST_MakeEnvelope($8,$9,$10,$11,{}))",
                    source.srid
                )
            } else {
                "NOT $7 AND CAST($8 AS double precision) IS NOT NULL AND CAST($9 AS double precision) IS NOT NULL AND CAST($10 AS double precision) IS NOT NULL AND CAST($11 AS double precision) IS NOT NULL".to_owned()
            };
            let source_key_filter = if r.feature_id.is_some() {
                format!("{key}=$6")
            } else {
                "CAST($6 AS text) IS NULL".to_owned()
            };
            format!(
                "SELECT {key} AS feature_id{source_payload} FROM {table} t LEFT JOIN gl_source_versions v ON v.project=$1 AND v.dataset=$2 AND v.feature_id=t.{id}::text COLLATE \"C\" WHERE coalesce(v.valid_from,{start})<=$3 AND {key}>$5 AND {source_key_filter} AND ({filter}) AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c WHERE c.project=$1 AND c.dataset=$2 AND c.workspace=$4 AND c.feature_id=t.{id}::text COLLATE \"C\") ORDER BY {key} LIMIT $12"
            )
        };
        let historical = if latest {
            String::new()
        } else {
            let sparse = self.sparse_spatial_candidates("history", r)?;
            let filter = self.spatial_filter("h", "history", r.bbox.is_some(), sparse);
            let lookup = if matches!(self.0.backend, Backend::Sqlite(_)) {
                " INDEXED BY sqlite_autoindex_gl_history_1"
            } else {
                ""
            };
            format!(
                ", historic AS MATERIALIZED (SELECT h.feature_id{payload} FROM gl_history h{lookup} WHERE h.project=$1 AND h.dataset=$2 AND h.valid_from<=$3 AND h.valid_to>$3 AND h.properties IS NOT NULL AND h.feature_id>$5 AND {key_filter} AND ({filter}) AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c WHERE c.project=$1 AND c.dataset=$2 AND c.workspace=$4 AND c.feature_id=h.feature_id) ORDER BY h.feature_id LIMIT $12)"
            )
        };
        let historic_union = if latest {
            ""
        } else {
            " UNION ALL SELECT * FROM historic"
        };
        let sql = format!(
            "WITH current AS MATERIALIZED ({current}){historical}, base AS MATERIALIZED (SELECT * FROM current{historic_union} ORDER BY feature_id LIMIT $12), draft AS MATERIALIZED (SELECT feature_id{draft_payload} FROM gl_workspace_changes{draft_lookup} WHERE project=$1 AND dataset=$2 AND workspace=$4 AND properties IS NOT NULL AND feature_id>$5 AND {draft_key_filter} AND ({draft_filter}) ORDER BY feature_id LIMIT $12), page AS MATERIALIZED (SELECT * FROM base UNION ALL SELECT * FROM draft ORDER BY feature_id LIMIT $12) SELECT * FROM page ORDER BY feature_id",
            draft_key_filter = if r.feature_id.is_some() {
                "feature_id=$6"
            } else {
                "CAST($6 AS text) IS NULL"
            }
        );
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
                "valid_from<=$3 AND valid_to>$3 AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c INDEXED BY sqlite_autoindex_gl_workspace_changes_1 WHERE c.project=$1 AND c.workspace=$4 AND c.dataset=$2 AND c.feature_id=prefix.feature_id)",
                ",valid_from,valid_to",
            )
        } else if table == "workspace_changes" {
            (
                "project=$1 AND workspace=$4 AND dataset=$2 AND feature_id>$5",
                "CAST($3 AS bigint) IS NOT NULL",
                "",
            )
        } else {
            (
                "feature_id>$5",
                "valid_from<=$3 AND NOT EXISTS(SELECT 1 FROM gl_workspace_changes c INDEXED BY sqlite_autoindex_gl_workspace_changes_1 WHERE c.project=$1 AND c.workspace=$4 AND c.dataset=$2 AND c.feature_id=prefix.feature_id)",
                ",valid_from",
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

pub(super) fn version_cells(row: Option<Row>) -> [Cell; 2] {
    row.map_or([Cell::Null, Cell::Null], |row| {
        [row.0[0].clone(), row.0[1].clone()]
    })
}
