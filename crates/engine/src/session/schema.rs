use super::*;
use crate::{DatasetSchema, SchemaField};

impl SqlTransaction {
    pub(super) fn read_schema_at(
        &mut self,
        project: &str,
        dataset: &str,
        revision: i64,
    ) -> Result<DatasetSchema> {
        if let Some(row)=self.query_opt("SELECT definition FROM gl_schema_history WHERE project=$1 AND dataset=$2 AND revision<=$3 ORDER BY revision DESC LIMIT 1", &[&project,&dataset,&revision])? {
            let definition: DatasetSchema = crate::codec::stored(&row.get::<_,String>(0usize)?)?;
            if !definition.dynamic {
                return Ok(definition);
            }
        }
        let metadata = self
            .dataset_exists(project, dataset)?
            .ok_or_else(crate::missing)?;
        if let Some(raw) = metadata.get::<_, Option<String>>(3usize)? {
            let source: super::tables::Source = crate::codec::stored(&raw)?;
            self.ensure_source_clean(project, dataset, &source)?;
            let rows=self.query("SELECT a.attname,format_type(a.atttypid,a.atttypmod),NOT a.attnotnull,a.attgenerated='' AND a.attidentity='' FROM pg_attribute a WHERE a.attrelid=to_regclass($1) AND a.attnum>0 AND NOT a.attisdropped AND a.attname<>$2 AND a.attname<>$3 ORDER BY a.attnum", &[&super::tables::relation(&source.table),&source.table.id_column,&source.table.geometry_column])?;
            let mut fields = Vec::new();
            for row in rows {
                fields.push(SchemaField {
                    name: row.get(0usize)?,
                    kind: row.get(1usize)?,
                    nullable: row.get(2usize)?,
                    editable: row.get(3usize)?,
                });
            }
            return Ok(DatasetSchema {
                supported: true,
                fields,
                reserved: vec![source.table.id_column, source.table.geometry_column],
                ..Default::default()
            });
        }
        let mut fields = std::collections::BTreeMap::<String, String>::new();
        let mut query = crate::repository::FeatureQuery {
            project: project.into(),
            dataset: dataset.into(),
            revision,
            workspace: None,
            after: String::new(),
            feature_id: None,
            bbox: None,
            limit: 32,
        };
        loop {
            let rows = self.feature_page(&query)?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                query.after = row.get(0usize)?;
                let properties: serde_json::Map<String, serde_json::Value> =
                    crate::codec::stored(&row.get::<_, String>(1usize)?)?;
                for (name, value) in properties {
                    let kind = if value.is_string() {
                        "text"
                    } else if value.is_boolean() {
                        "boolean"
                    } else if value.is_i64() {
                        "bigint"
                    } else if value.is_number() {
                        "double precision"
                    } else {
                        "jsonb"
                    };
                    fields
                        .entry(name)
                        .and_modify(|existing| {
                            if existing != kind {
                                *existing = "jsonb".into();
                            }
                        })
                        .or_insert_with(|| kind.into());
                    if fields.len() > 256 {
                        return Err(Error::new(
                            413,
                            "versioned schemas support at most 256 property fields",
                        ));
                    }
                }
            }
        }
        Ok(DatasetSchema {
            supported: true,
            dynamic: true,
            fields: fields
                .into_iter()
                .map(|(name, kind)| SchemaField {
                    name,
                    kind,
                    nullable: true,
                    editable: true,
                })
                .collect(),
            ..Default::default()
        })
    }

    pub(super) fn read_workspace_schemas(
        &mut self,
        project: &str,
        workspace: &str,
    ) -> Result<Vec<(String, DatasetSchema)>> {
        self.query("SELECT dataset,definition FROM gl_workspace_schemas WHERE project=$1 AND workspace=$2 ORDER BY dataset", &[&project,&workspace])?.into_iter().map(|r|Ok((r.get(0usize)?,crate::codec::stored(&r.get::<_,String>(1usize)?)?))).collect()
    }

    pub(super) fn replace_schema_intent(
        &mut self,
        project: &str,
        workspace: &str,
        definitions: &[(String, DatasetSchema, DatasetSchema)],
    ) -> Result<()> {
        let base: i64 = self
            .query_one(
                "SELECT base_revision FROM gl_workspaces WHERE project=$1 AND id=$2",
                &[&project, &workspace],
            )?
            .get(0usize)?;
        for (dataset, previous) in self.read_workspace_schemas(project, workspace)? {
            let original = self.read_schema_at(project, &dataset, base)?;
            let next = definitions
                .iter()
                .find(|(id, _, _)| *id == dataset)
                .map(|(_, _, next)| next)
                .unwrap_or(&original);
            let removed: std::collections::BTreeSet<_> = previous
                .fields
                .iter()
                .filter(|field| {
                    !original.fields.iter().any(|base| base.name == field.name)
                        && !next.fields.iter().any(|next| next.name == field.name)
                })
                .map(|field| field.name.clone())
                .collect();
            if removed.is_empty() {
                continue;
            }
            let mut after = String::new();
            loop {
                let rows = self.query(&format!("SELECT feature_id,properties,geometry_json,resolved_head IS NOT NULL FROM {} WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id>$4 AND properties IS NOT NULL ORDER BY feature_id LIMIT 32", self.workspace_key_table()), &[&project,&workspace,&dataset,&after])?;
                if rows.is_empty() {
                    break;
                }
                for row in rows {
                    after = row.get(0usize)?;
                    let mut properties: serde_json::Map<String, serde_json::Value> =
                        crate::codec::stored(&row.get::<_, String>(1usize)?)?;
                    let old_len = properties.len();
                    properties.retain(|name, _| !removed.contains(name));
                    if properties.len() == old_len {
                        continue;
                    }
                    let geometry: Option<String> = row.get(2usize)?;
                    let value = crate::feature::Stored {
                        properties: properties.clone(),
                        geometry: geometry.clone(),
                    };
                    let original =
                        FeatureRepository::feature_at(self, project, &dataset, &after, base)?
                            .map(|row| crate::feature::stored_row(&row, 0, 1))
                            .transpose()?
                            .flatten();
                    if !row.get::<_, bool>(3usize)? && original.as_ref() == Some(&value) {
                        WorkspaceRepository::remove_delta(
                            self, project, workspace, &dataset, &after,
                        )?;
                    } else {
                        let properties =
                            serde_json::to_string(&properties).map_err(Error::stored_json)?;
                        self.execute(&format!("UPDATE {} SET properties=$5 WHERE project=$1 AND workspace=$2 AND dataset=$3 AND feature_id=$4", self.workspace_key_table()), &[&project,&workspace,&dataset,&after,&properties])?;
                    }
                }
            }
        }
        self.execute(
            "DELETE FROM gl_workspace_schemas WHERE project=$1 AND workspace=$2",
            &[&project, &workspace],
        )?;
        for (dataset, base, definition) in definitions {
            let base = serde_json::to_string(base).map_err(Error::stored_json)?;
            let definition = serde_json::to_string(definition).map_err(Error::stored_json)?;
            self.execute("INSERT INTO gl_schema_history(project,dataset,revision,definition) VALUES($1,$2,0,$3) ON CONFLICT(project,dataset,revision) DO NOTHING", &[&project,&dataset,&base])?;
            self.execute("INSERT INTO gl_workspace_schemas(project,workspace,dataset,definition) VALUES($1,$2,$3,$4)", &[&project,&workspace,&dataset,&definition])?;
        }
        Ok(())
    }

    pub(super) fn materialize_schema_merge(
        &mut self,
        project: &str,
        workspace: &str,
        current: i64,
    ) -> Result<usize> {
        self.flush_stage()?;
        let before: i64 = self
            .query_one("SELECT count(*) FROM gl_merge_stage", &[])?
            .get(0usize)?;
        for (dataset, mut definition) in self.read_workspace_schemas(project, workspace)? {
            let original = self.schema_projection(project, &dataset, current)?;
            definition
                .dropped
                .retain(|name| !original.dropped.contains(name));
            let mut query = crate::repository::FeatureQuery {
                project: project.into(),
                dataset: dataset.clone(),
                revision: current,
                workspace: None,
                after: String::new(),
                feature_id: None,
                bbox: None,
                limit: 32,
            };
            loop {
                let rows = self.feature_page(&query)?;
                if rows.is_empty() {
                    break;
                }
                for row in rows {
                    let key: String = row.get(0usize)?;
                    query.after = key.clone();
                    let old = crate::feature::stored_row(&row, 1, 2)?.ok_or_else(crate::bad)?;
                    let staged = self.query_opt(
                        "SELECT after_value FROM gl_merge_stage WHERE dataset=$1 AND feature_id=$2",
                        &[&dataset, &key],
                    )?;
                    let mut value = match staged {
                        Some(row) => match row.get::<_, Option<String>>(0usize)? {
                            Some(v) => crate::codec::stored::<crate::feature::Stored>(&v)?,
                            None => continue,
                        },
                        None => old.clone(),
                    };
                    crate::schema::project_properties(&mut value.properties, &definition);
                    crate::feature::validate_candidate(&value)?;
                    crate::schema::validate_properties(&value.properties, &definition)?;
                    if value != old {
                        let old = serde_json::to_string(&old).map_err(Error::stored_json)?;
                        let value = serde_json::to_string(&value).map_err(Error::stored_json)?;
                        self.execute("INSERT INTO gl_merge_stage(dataset,feature_id,before_value,after_value) VALUES($1,$2,$3,$4) ON CONFLICT(dataset,feature_id) DO UPDATE SET after_value=excluded.after_value", &[&dataset,&key,&old,&value])?;
                    }
                }
            }
            let mut after = String::new();
            loop {
                let rows=self.query("SELECT feature_id,after_value FROM gl_merge_stage WHERE dataset=$1 AND feature_id>$2 AND after_value IS NOT NULL ORDER BY feature_id LIMIT 32", &[&dataset,&after])?;
                if rows.is_empty() {
                    break;
                }
                for row in rows {
                    after = row.get(0usize)?;
                    let mut value: crate::feature::Stored =
                        crate::codec::stored(&row.get::<_, String>(1usize)?)?;
                    crate::schema::project_properties(&mut value.properties, &definition);
                    crate::feature::validate_candidate(&value)?;
                    crate::schema::validate_properties(&value.properties, &definition)?;
                    let value = serde_json::to_string(&value).map_err(Error::stored_json)?;
                    self.execute("UPDATE gl_merge_stage SET after_value=$3 WHERE dataset=$1 AND feature_id=$2", &[&dataset,&after,&value])?;
                }
            }
        }
        let count: i64 = self
            .query_one("SELECT count(*) FROM gl_merge_stage", &[])?
            .get(0usize)?;
        Ok(count.saturating_sub(before) as usize)
    }

    pub(super) fn publish_schema_definitions(
        &mut self,
        project: &str,
        revision: i64,
    ) -> Result<std::collections::BTreeSet<String>> {
        let workspace: String = self
            .query_one(
                "SELECT workspace FROM gl_commits WHERE project=$1 AND revision=$2",
                &[&project, &revision],
            )?
            .get(0usize)?;
        let mut prepared = std::collections::BTreeSet::new();
        for (dataset, definition) in self.read_workspace_schemas(project, &workspace)? {
            let metadata = self
                .dataset_exists(project, &dataset)?
                .ok_or_else(crate::missing)?;
            if let Some(raw) = metadata.get::<_, Option<String>>(3usize)? {
                let source: super::tables::Source = crate::codec::stored(&raw)?;
                let actual = self.inspect_source(&source.table)?;
                if source.signature != actual.signature {
                    return Err(Error::new(409, "business table schema drift"));
                }
                self.ensure_source_clean(project, &dataset, &source)?;
                self.check_source(&dataset, &source, false)?;
                let table = super::tables::relation(&source.table);
                let current = self.read_schema_at(project, &dataset, revision - 1)?;
                for field in &current.fields {
                    if definition.fields.iter().any(|f| f.name == field.name) {
                        continue;
                    }
                    let blocked:bool=self.query_one("SELECT a.attnotnull OR a.attgenerated<>'' OR a.attidentity<>'' OR EXISTS(SELECT 1 FROM pg_attrdef d WHERE d.adrelid=a.attrelid AND d.adnum=a.attnum) OR EXISTS(SELECT 1 FROM pg_index i WHERE i.indrelid=a.attrelid AND a.attnum=ANY(i.indkey)) OR EXISTS(SELECT 1 FROM pg_depend d WHERE d.refclassid='pg_class'::regclass AND d.refobjid=a.attrelid AND d.refobjsubid=a.attnum) FROM pg_attribute a WHERE a.attrelid=to_regclass($1) AND a.attname=$2 AND NOT a.attisdropped", &[&table,&field.name])?.get(0usize)?;
                    if blocked
                        || field.name == source.table.id_column
                        || field.name == source.table.geometry_column
                        || sql_type(&field.kind).is_none()
                    {
                        return Err(Error::new(
                            400,
                            "field has protected constraints, dependencies or an unsupported restorable type",
                        ));
                    }
                    self.batch_execute(&format!(
                        "ALTER TABLE {table} DROP COLUMN {}",
                        super::tables::quote(&field.name)
                    ))?;
                }
                for field in &definition.fields {
                    if current.fields.iter().any(|f| f.name == field.name) {
                        continue;
                    }
                    let kind = sql_type(&field.kind)
                        .ok_or_else(|| Error::new(400, "unsupported schema field type"))?;
                    self.batch_execute(&format!(
                        "ALTER TABLE {table} ADD COLUMN {} {kind}",
                        super::tables::quote(&field.name)
                    ))?;
                }
                let mut updated = self.inspect_source(&source.table)?;
                if updated.srid != source.srid
                    || updated.dimension != source.dimension
                    || updated.family != source.family
                {
                    return Err(Error::new(409, "schema edit changed the geometry contract"));
                }
                updated.order_index = source.order_index.clone();
                let encoded = serde_json::to_string(&updated).map_err(Error::stored_json)?;
                let (stamp, binding_definition) = self.source_stamp(&source.table)?;
                self.execute(
                    "UPDATE gl_datasets SET postgis_source=$3 WHERE project=$1 AND id=$2",
                    &[&project, &dataset, &encoded],
                )?;
                self.execute("UPDATE gl_source_state SET stamp=$3,definition=$4 WHERE schema_name=$1 AND table_name=$2", &[&source.table.schema,&source.table.table,&stamp,&binding_definition])?;
                self.1.datasets.remove(&(project.into(), dataset.clone()));
                self.1
                    .verified_sources
                    .remove(&(project.into(), dataset.clone()));
                prepared.insert(dataset.clone());
            }
            let encoded = serde_json::to_string(&definition).map_err(Error::stored_json)?;
            self.execute("INSERT INTO gl_schema_history(project,dataset,revision,definition) VALUES($1,$2,$3,$4)", &[&project,&dataset,&revision,&encoded])?;
        }
        Ok(prepared)
    }

    pub(super) fn restore_schema_definitions(
        &mut self,
        project: &str,
        workspace: &str,
        revision: i64,
        current: i64,
    ) -> Result<()> {
        let rows=self.query("SELECT DISTINCT dataset FROM gl_schema_history WHERE project=$1 AND revision>$2 AND revision<=$3 ORDER BY dataset", &[&project,&revision,&current])?;
        let mut definitions = Vec::new();
        for row in rows {
            let dataset: String = row.get(0usize)?;
            let base = self.read_schema_at(project, &dataset, current)?;
            let restored = self.read_schema_at(project, &dataset, revision)?;
            if base != restored {
                definitions.push((dataset, base, restored));
            }
        }
        self.replace_schema_intent(project, workspace, &definitions)
    }
}

fn sql_type(kind: &str) -> Option<&str> {
    match kind {
        "text" => Some("text"),
        "int2" | "smallint" => Some("smallint"),
        "int4" | "integer" => Some("integer"),
        "int8" | "bigint" => Some("bigint"),
        "float4" | "real" => Some("real"),
        "float8" | "double precision" => Some("double precision"),
        "bool" | "boolean" => Some("boolean"),
        "json" => Some("json"),
        "jsonb" => Some("jsonb"),
        _ => None,
    }
}
