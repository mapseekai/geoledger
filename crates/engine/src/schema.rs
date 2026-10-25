use super::*;
use crate::repository::{DatasetRepository, ProjectRepository, WorkspaceRepository};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SchemaField {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub nullable: bool,
    pub editable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct DatasetSchema {
    pub supported: bool,
    pub fields: Vec<SchemaField>,
    pub reserved: Vec<String>,
    pub defaults: BTreeSet<String>,
    pub dropped: BTreeSet<String>,
    pub dynamic: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddField {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaEdit {
    pub dataset: String,
    #[serde(default)]
    pub add: Vec<AddField>,
    #[serde(default)]
    pub drop: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SchemaQuery {
    pub project: String,
    pub dataset: String,
    pub revision: Option<i64>,
    pub workspace: Option<String>,
}

pub(super) fn schema_query(
    t: &mut (impl DatasetRepository + ProjectRepository + WorkspaceRepository + ?Sized),
    subject: &str,
    r: SchemaQuery,
) -> Result<Value> {
    membership(t, &r.project, subject, false)?;
    dataset(t, &r.project, &r.dataset)?;
    if r.workspace.is_some() && r.revision.is_some() {
        return Err(bad());
    }
    let (revision, version) = if let Some(w) = &r.workspace {
        let (base, version, status) = workspace(t, subject, &r.project, w, None)?;
        let revision = if status == "published" {
            t.workspace_publication_revision(&r.project, w)?
                .ok_or_else(missing)?
        } else {
            base
        };
        (revision, Some(version))
    } else {
        let h = head(t, &r.project, false)?;
        let revision = r.revision.unwrap_or(h);
        if revision < 0 || revision > h {
            return Err(bad());
        }
        (revision, None)
    };
    let definition = effective_schema(t, &r.project, &r.dataset, revision, r.workspace.as_deref())?;
    if !definition.supported {
        return Err(Error::new(
            400,
            "storage does not support versioned schemas",
        ));
    }
    Ok(json!({"revision":revision,"workspace_version":version,"fields":definition.fields}))
}

pub(super) fn effective_schema(
    t: &mut (impl DatasetRepository + ?Sized),
    project: &str,
    dataset: &str,
    revision: i64,
    workspace: Option<&str>,
) -> Result<DatasetSchema> {
    if let Some(w) = workspace {
        for (id, definition) in t.workspace_schemas(project, w)? {
            if id == dataset {
                return Ok(definition);
            }
        }
    }
    t.schema_at(project, dataset, revision)
}

pub(super) fn effective_projection(
    t: &mut (impl DatasetRepository + ?Sized),
    project: &str,
    dataset: &str,
    revision: i64,
    workspace: Option<&str>,
) -> Result<DatasetSchema> {
    if let Some(w) = workspace {
        for (id, definition) in t.workspace_schemas(project, w)? {
            if id == dataset {
                return Ok(definition);
            }
        }
    }
    t.schema_projection(project, dataset, revision)
}

pub(super) fn save_schema_edits(
    t: &mut (impl DatasetRepository + ProjectRepository + ?Sized),
    project: &str,
    workspace: &str,
    base: i64,
    edits: Vec<SchemaEdit>,
) -> Result<()> {
    if edits.len() > 100 {
        return Err(bad());
    }
    let mut seen = BTreeSet::new();
    let mut definitions = Vec::new();
    for edit in edits {
        if !seen.insert(edit.dataset.clone()) || edit.add.len() + edit.drop.len() > 256 {
            return Err(bad());
        }
        dataset(t, project, &edit.dataset)?;
        let original = t.schema_at(project, &edit.dataset, base)?;
        if !original.supported {
            return Err(Error::new(
                400,
                "storage does not support versioned schemas",
            ));
        }
        let mut target = original.clone();
        let mut names = BTreeSet::new();
        for name in edit.drop {
            text(&name, 63)?;
            if !names.insert(name.clone()) || original.reserved.contains(&name) {
                return Err(bad());
            }
            let field = target
                .fields
                .iter()
                .find(|f| f.name == name)
                .ok_or_else(bad)?;
            if !field.editable || !field.nullable {
                return Err(Error::new(
                    400,
                    "only ordinary nullable property fields can be dropped",
                ));
            }
            target.fields.retain(|f| f.name != name);
            target.defaults.remove(&name);
            target.dropped.insert(name);
        }
        for field in edit.add {
            text(&field.name, 63)?;
            if matches!(
                field.name.as_str(),
                "$serde_json::private::RawValue" | "$serde_json::private::Number"
            ) || !names.insert(field.name.clone())
                || original.reserved.contains(&field.name)
                || target.fields.iter().any(|f| f.name == field.name)
            {
                return Err(bad());
            }
            let kind = match field.kind.as_str() {
                "string" => "text",
                "number" => "float8",
                "integer" => "int8",
                "boolean" => "bool",
                "json" => "jsonb",
                _ => {
                    return Err(Error::new(
                        400,
                        "field type must be string, number, integer, boolean or json",
                    ));
                }
            };
            target.defaults.insert(field.name.clone());
            target.dropped.remove(&field.name);
            target.fields.push(SchemaField {
                name: field.name,
                kind: kind.into(),
                nullable: true,
                editable: true,
            });
        }
        if target.fields.len() > 256 {
            return Err(bad());
        }
        if target != original {
            target.dynamic = false;
            definitions.push((edit.dataset, original, target));
        }
    }
    // save already holds the project edit SHARE lock until commit. Reading
    // HEAD here keeps publication excluded without upgrading two editors' locks.
    if !definitions.is_empty() && head(t, project, false)? != base {
        return Err(Error::new(
            409,
            "schema edits require the current published base; retain the draft and rebase before changing fields",
        ));
    }
    t.replace_workspace_schemas(project, workspace, &definitions)
}

pub(super) fn project_properties(properties: &mut Map<String, Value>, schema: &DatasetSchema) {
    for name in &schema.dropped {
        properties.remove(name);
    }
    for name in &schema.defaults {
        properties.entry(name.clone()).or_insert(Value::Null);
    }
}

pub(super) fn validate_properties(
    properties: &Map<String, Value>,
    schema: &DatasetSchema,
) -> Result<()> {
    if schema.dynamic {
        return Ok(());
    }
    for (name, value) in properties {
        let field = schema
            .fields
            .iter()
            .find(|f| f.name == *name)
            .ok_or_else(|| Error::new(400, "feature properties must match the versioned schema"))?;
        if value.is_null() {
            if !field.nullable {
                return Err(Error::new(400, "field is not nullable"));
            }
            continue;
        }
        let valid = match field.kind.as_str() {
            "text" | "varchar" => value.is_string(),
            "float8" | "float4" | "numeric" | "double precision" | "real" => value.is_number(),
            "int2" | "int4" | "int8" | "smallint" | "integer" | "bigint" => {
                value.as_i64().is_some()
            }
            "bool" | "boolean" => value.is_boolean(),
            _ => true,
        };
        if !valid {
            return Err(Error::new(
                400,
                "property value does not match the versioned field type",
            ));
        }
    }
    Ok(())
}

pub(super) fn check_schema_head(
    t: &mut (impl DatasetRepository + ?Sized),
    project: &str,
    workspace: &str,
    base: i64,
    current: i64,
) -> Result<()> {
    if base != current
        && (!t.workspace_schemas(project, workspace)?.is_empty()
            || t.schema_changed_since(project, base, current)?)
    {
        return Err(Error::new(
            409,
            "schema changed against this base; retain the draft and explicitly reconcile its schema",
        ));
    }
    Ok(())
}

pub(super) fn workspace_schema_diff(
    t: &mut (impl DatasetRepository + ?Sized),
    project: &str,
    workspace: &str,
    base: i64,
) -> Result<Value> {
    let mut result = Vec::new();
    for (dataset, definition) in t.workspace_schemas(project, workspace)? {
        result.push(json!({"dataset":dataset,"before":t.schema_at(project,&dataset,base)?.fields,"after":definition.fields}));
    }
    Ok(Value::Array(result))
}
pub(super) fn commit_schema_diff(
    t: &mut (impl DatasetRepository + ?Sized),
    project: &str,
    revision: i64,
) -> Result<Value> {
    Ok(json!(t.published_schema_changes(project,revision)?.into_iter().map(|(dataset,before,after)|json!({"dataset":dataset,"before":before.fields,"after":after.fields})).collect::<Vec<_>>()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test fixture setup and assertions fail immediately"
)]
mod tests {
    use super::*;

    #[test]
    fn schema_intent_is_reversible_versioned_and_restorable() {
        let dir = tempfile::tempdir().unwrap();
        let app = Application::new(Storage::Sqlite(dir.path().join("schema.db")));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let project = call("create_project", json!({"name":"schema"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let dataset=call("create_dataset",json!({"project":project,"name":"points","geometry_type":"point","coordinate_dimension":3,"crs":"EPSG:3857"}))["dataset"].as_str().unwrap().to_owned();
        let first = call("create_workspace", json!({"project":project}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let feature = json!({"type":"Feature","id":"a","properties":{"name":"first","remove":"original","optional":null},"geometry":{"type":"Point","coordinates":[10.123456789,20.987654321,30.75]}});
        call(
            "save",
            json!({"project":project,"workspace":first,"expected_workspace_version":0,"edits":[{"dataset":dataset,"feature_id":"a","feature":feature}]}),
        );
        call(
            "publish",
            json!({"project":project,"workspace":first,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"baseline"}),
        );
        let workspace = call("create_workspace", json!({"project":project}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let intent = json!([{"dataset":dataset,"add":[{"name":"extra","type":"integer"}],"drop":["remove"]}]);
        call(
            "save",
            json!({"project":project,"workspace":workspace,"expected_workspace_version":0,"schema_edits":intent}),
        );
        let working = call(
            "features",
            json!({"project":project,"dataset":dataset,"workspace":workspace}),
        );
        assert!(working["features"][0]["properties"].get("remove").is_none());
        assert_eq!(working["features"][0]["properties"]["extra"], Value::Null);
        let schema = call(
            "schema",
            json!({"project":project,"dataset":dataset,"workspace":workspace}),
        );
        assert_eq!(schema["revision"], 1);
        assert_eq!(schema["workspace_version"], 1);
        assert!(
            schema["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == "extra")
        );
        let diff = call("diff", json!({"project":project,"workspace":workspace}));
        assert_eq!(diff["schema_changes"].as_array().unwrap().len(), 1);
        call(
            "save",
            json!({"project":project,"workspace":workspace,"expected_workspace_version":1,"schema_edits":[]}),
        );
        let cleared = call(
            "features",
            json!({"project":project,"dataset":dataset,"workspace":workspace}),
        );
        assert_eq!(cleared["features"][0], feature);
        call(
            "save",
            json!({"project":project,"workspace":workspace,"expected_workspace_version":2,"schema_edits":intent}),
        );
        let mut edited = working["features"][0].clone();
        edited["properties"]["name"] = json!("edited");
        call(
            "save",
            json!({"project":project,"workspace":workspace,"expected_workspace_version":3,"edits":[{"dataset":dataset,"feature_id":"a","feature":edited}]}),
        );
        let publication = json!({"project":project,"workspace":workspace,"expected_workspace_version":4,"request_id":Uuid::new_v4(),"message":"schema"});
        let result = call("publish", publication.clone());
        assert_eq!(result, call("publish", publication));
        assert_eq!(result["revision"], 2);
        let historical = call(
            "features",
            json!({"project":project,"dataset":dataset,"revision":1}),
        );
        assert_eq!(historical["features"][0], feature);
        let oldschema = call(
            "schema",
            json!({"project":project,"dataset":dataset,"revision":1}),
        );
        assert!(
            !oldschema["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["name"] == "extra")
        );
        let summary = call("commit_summary", json!({"project":project,"revision":2}));
        assert_eq!(summary["schema_changes"].as_array().unwrap().len(), 1);
        let restored = call("restore", json!({"project":project,"revision":2}));
        let w = restored["workspace"].as_str().unwrap();
        call(
            "publish",
            json!({"project":project,"workspace":w,"expected_workspace_version":0,"request_id":Uuid::new_v4(),"message":"restore schema"}),
        );
        assert_eq!(
            call("features", json!({"project":project,"dataset":dataset}))["features"][0],
            feature
        );
        let mut export = Vec::new();
        app.export_data(&mut export).unwrap();
        let imported = Application::new(Storage::Sqlite(dir.path().join("imported.db")));
        imported.migrate().unwrap();
        imported
            .import_data(&mut std::io::Cursor::new(export))
            .unwrap();
        assert_eq!(
            imported
                .execute(
                    "alice",
                    "schema",
                    json!({"project":project,"dataset":dataset,"revision":1})
                )
                .unwrap(),
            oldschema
        );
        assert_eq!(
            imported
                .execute(
                    "alice",
                    "features",
                    json!({"project":project,"dataset":dataset})
                )
                .unwrap()["features"][0],
            feature
        );
    }

    #[test]
    fn schema_conflict_rejects_without_losing_the_intent() {
        let dir = tempfile::tempdir().unwrap();
        let app = Application::new(Storage::Sqlite(dir.path().join("conflict.db")));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let p = call("create_project", json!({"name":"conflict"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = call(
            "create_dataset",
            json!({"project":p,"name":"points","geometry_type":"point"}),
        )["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"add":[{"name":"new","type":"string"}]}]}),
        );
        let other = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":other,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"x","feature":{"type":"Feature","id":"x","properties":{},"geometry":null}}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":other,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"move head"}),
        );
        let failed=app.execute("alice","publish",json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"stale schema"})).unwrap_err();
        assert_eq!(failed.status, 409);
        let schema = call("schema", json!({"project":p,"dataset":d,"workspace":w}));
        assert_eq!(schema["workspace_version"], 1);
        assert_eq!(schema["fields"][0]["name"], "new");
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"schema_edits":[]}),
        );
        assert!(
            call("schema", json!({"project":p,"dataset":d,"workspace":w}))["fields"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn schema_clear_removes_temporary_fields_and_recovers_edited_hidden_values() {
        let dir = tempfile::tempdir().unwrap();
        let app = Application::new(Storage::Sqlite(dir.path().join("clear.db")));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let p = call("create_project", json!({"name":"clear"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = call(
            "create_dataset",
            json!({"project":p,"name":"points","geometry_type":"point"}),
        )["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut feature = json!({"type":"Feature","id":"a","properties":{"keep":"before","gone":"original","nullable":null},"geometry":null});
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"base"}),
        );
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        feature["properties"]["gone"] = json!("edited before hiding");
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature}]}),
        );
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"schema_edits":[{"dataset":d,"add":[{"name":"temporary","type":"integer"}],"drop":["gone"]}]}),
        );
        let mut projected =
            call("features", json!({"project":p,"dataset":d,"workspace":w}))["features"][0].clone();
        projected["properties"]["keep"] = json!("after");
        projected["properties"]["temporary"] = json!(7);
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":2,"edits":[{"dataset":d,"feature_id":"a","feature":projected}]}),
        );
        let mut malicious = projected;
        malicious["properties"]["gone"] = json!("hidden overwrite");
        let error=app.execute("alice","save",json!({"project":p,"workspace":w,"expected_workspace_version":3,"edits":[{"dataset":d,"feature_id":"a","feature":malicious}]})).unwrap_err();
        assert_eq!(error.status, 400);
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":3,"schema_edits":[]}),
        );
        feature["properties"]["keep"] = json!("after");
        assert_eq!(
            call("features", json!({"project":p,"dataset":d,"workspace":w}))["features"][0],
            feature
        );
        let result = call(
            "publish",
            json!({"project":p,"workspace":w,"expected_workspace_version":4,"request_id":Uuid::new_v4(),"message":"only feature edit"}),
        );
        assert_eq!(
            call(
                "commit_summary",
                json!({"project":p,"revision":result["revision"]})
            )["schema_changes"],
            json!([])
        );
        assert_eq!(
            call("features", json!({"project":p,"dataset":d}))["features"][0],
            feature
        );
    }

    #[test]
    fn schema_history_before_first_explicit_schema_uses_each_revision_properties() {
        let dir = tempfile::tempdir().unwrap();
        let app = Application::new(Storage::Sqlite(dir.path().join("dynamic.db")));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let p = call("create_project", json!({"name":"dynamic"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = call(
            "create_dataset",
            json!({"project":p,"name":"points","geometry_type":"point"}),
        )["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        for (index, properties) in [json!({"first":"one"}), json!({"first":"two","later":null})]
            .into_iter()
            .enumerate()
        {
            let w = call("create_workspace", json!({"project":p}))["workspace"]
                .as_str()
                .unwrap()
                .to_owned();
            call(
                "save",
                json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":{"type":"Feature","id":"a","properties":properties,"geometry":null}}]}),
            );
            call(
                "publish",
                json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":format!("revision {index}")}),
            );
        }
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        for name in [
            "$serde_json::private::RawValue",
            "$serde_json::private::Number",
        ] {
            assert_eq!(app.execute("alice","save",json!({"project":p,"workspace":w,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"add":[{"name":name,"type":"string"}]}]})).unwrap_err().status,400);
        }
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"add":[{"name":"explicit","type":"boolean"}]}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"explicit schema"}),
        );
        assert_eq!(
            call("schema", json!({"project":p,"dataset":d,"revision":0}))["fields"],
            json!([])
        );
        let fields = call("schema", json!({"project":p,"dataset":d,"revision":1}))["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(fields, vec!["first"]);
        assert_eq!(
            call("schema", json!({"project":p,"dataset":d,"revision":2}))["fields"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn schema_resolution_cannot_reintroduce_a_retired_property() {
        let dir = tempfile::tempdir().unwrap();
        let app = Application::new(Storage::Sqlite(dir.path().join("resolution.db")));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let p = call("create_project", json!({"name":"resolution"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = call(
            "create_dataset",
            json!({"project":p,"name":"points","geometry_type":"point"}),
        )["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        let base = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut feature = json!({"type":"Feature","id":"a","properties":{"keep":"one","retired":"original"},"geometry":null});
        call(
            "save",
            json!({"project":p,"workspace":base,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":base,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"base"}),
        );
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        feature["properties"]["retired"] = json!("local edit");
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature}]}),
        );
        let other = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":other,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"drop":["retired"]}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":other,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"retire property"}),
        );
        for op in ["resolve", "rebase"] {
            let error=app.execute("alice",op,json!({"project":p,"workspace":w,"expected_workspace_version":1,"expected_head":2,"resolutions":[{"dataset":d,"feature_id":"a","feature":feature}]})).unwrap_err();
            assert_eq!(
                error.status, 400,
                "{op} must validate the current field contract"
            );
        }
        feature["properties"]
            .as_object_mut()
            .unwrap()
            .remove("retired");
        feature["properties"]["keep"] = json!("resolved");
        call(
            "rebase",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"expected_head":2,"resolutions":[{"dataset":d,"feature_id":"a","feature":feature}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":w,"expected_workspace_version":2,"request_id":Uuid::new_v4(),"message":"reconciled"}),
        );
        assert_eq!(
            call("features", json!({"project":p,"dataset":d}))["features"][0],
            feature
        );
    }

    #[test]
    fn schema_resolve_preserves_hidden_values_when_head_moves() {
        let dir = tempfile::tempdir().unwrap();
        let app = Application::new(Storage::Sqlite(dir.path().join("resolve-hidden.db")));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let p = call("create_project", json!({"name":"resolve-hidden"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = call(
            "create_dataset",
            json!({"project":p,"name":"points","geometry_type":"point"}),
        )["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        let base = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let feature = |keep: &str, gone: &str| json!({"type":"Feature","id":"a","properties":{"keep":keep,"gone":gone},"geometry":null});
        call(
            "save",
            json!({"project":p,"workspace":base,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature("base","original")}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":base,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"base"}),
        );
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature("local","edited before hiding")}]}),
        );
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"schema_edits":[{"dataset":d,"drop":["gone"]}]}),
        );
        let remote = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":remote,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature("remote","original")}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":remote,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"move head"}),
        );
        let before = call("get_workspace", json!({"project":p,"workspace":w}));
        let schema = call("schema", json!({"project":p,"dataset":d,"workspace":w}));
        let rebase=app.execute("alice","rebase",json!({"project":p,"workspace":w,"expected_workspace_version":2,"expected_head":2,"resolutions":[]})).unwrap_err();
        assert_eq!(rebase.status, 409);
        let resolution =
            json!({"type":"Feature","id":"a","properties":{"keep":"resolved"},"geometry":null});
        let error=app.execute("alice","resolve",json!({"project":p,"workspace":w,"expected_workspace_version":2,"expected_head":2,"resolutions":[{"dataset":d,"feature_id":"a","feature":resolution}]})).unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(
            call("get_workspace", json!({"project":p,"workspace":w})),
            before
        );
        assert_eq!(
            call("schema", json!({"project":p,"dataset":d,"workspace":w})),
            schema
        );
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":2,"schema_edits":[]}),
        );
        assert_eq!(
            call("features", json!({"project":p,"dataset":d,"workspace":w}))["features"][0],
            feature("local", "edited before hiding")
        );
        assert_eq!(call("get_project", json!({"project":p}))["head"], 2);
    }

    #[test]
    fn schema_merge_validates_automatic_and_current_base_publication() {
        let dir = tempfile::tempdir().unwrap();
        let app = Application::new(Storage::Sqlite(dir.path().join("automatic-schema.db")));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let p = call("create_project", json!({"name":"automatic-schema"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = call(
            "create_dataset",
            json!({"project":p,"name":"points","geometry_type":"point"}),
        )["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        let base = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let feature = |id: &str, properties: Value| json!({"type":"Feature","id":id,"properties":properties,"geometry":null});
        call(
            "save",
            json!({"project":p,"workspace":base,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature("a",json!({"keep":"base"}))}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":base,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"base"}),
        );
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let invalid = feature("b", json!({"keep":"new local","extra":"not an integer"}));
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"b","feature":invalid}]}),
        );
        let remote = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":remote,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"add":[{"name":"extra","type":"integer"}]}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":remote,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"typed field"}),
        );
        let before = call("get_workspace", json!({"project":p,"workspace":w}));
        let error=app.execute("alice","rebase",json!({"project":p,"workspace":w,"expected_workspace_version":1,"expected_head":2,"resolutions":[]})).unwrap_err();
        assert_eq!(error.status, 400);
        assert_eq!(
            call("get_workspace", json!({"project":p,"workspace":w})),
            before
        );
        assert_eq!(
            call("features", json!({"project":p,"dataset":d,"workspace":w}))["features"][1],
            invalid
        );
        let blocked=app.execute("alice","publish",json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"old schema"})).unwrap_err();
        assert_eq!(blocked.status, 409);

        // A low-level fixture models a persisted/current-base draft from a
        // pre-fix import or backend. Publication must independently reject it.
        let current = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut transaction = app.storage.begin(false, Duration::from_secs(30)).unwrap();
        transaction
            .put_deltas(
                &p,
                &current,
                &[repository::DraftDelta {
                    dataset: d.clone(),
                    key: "c".into(),
                    properties: Some(
                        json!({"keep":"persisted","extra":"not an integer"}).to_string(),
                    ),
                    geometry: None,
                }],
            )
            .unwrap();
        transaction.commit().unwrap();
        let current_before = call("get_workspace", json!({"project":p,"workspace":current}));
        let rejected=app.execute("alice","publish",json!({"project":p,"workspace":current,"expected_workspace_version":0,"request_id":Uuid::new_v4(),"message":"invalid current base"})).unwrap_err();
        assert_eq!(rejected.status, 400);
        assert_eq!(
            call("get_workspace", json!({"project":p,"workspace":current})),
            current_before
        );
        assert_eq!(call("get_project", json!({"project":p}))["head"], 2);
        assert_eq!(
            call(
                "features",
                json!({"project":p,"dataset":d,"workspace":current})
            )["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["id"] == "c")
                .unwrap()["properties"]["extra"],
            "not an integer"
        );

        let repaired = feature("b", json!({"keep":"new local","extra":7}));
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"edits":[{"dataset":d,"feature_id":"b","feature":repaired}]}),
        );
        call(
            "rebase",
            json!({"project":p,"workspace":w,"expected_workspace_version":2,"expected_head":2,"resolutions":[]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":w,"expected_workspace_version":3,"request_id":Uuid::new_v4(),"message":"explicit reconciliation"}),
        );
        assert_eq!(
            call(
                "features",
                json!({"project":p,"dataset":d,"feature_id":"b"})
            )["properties"]["extra"],
            7
        );
    }

    #[test]
    fn schema_field_replacement_counts_only_visible_properties() {
        for count in [255, 256] {
            let mut outcomes = Vec::new();
            for with_feature in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let app = Application::new(Storage::Sqlite(dir.path().join("field-limit.db")));
                app.migrate().unwrap();
                let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
                let p = call("create_project", json!({"name":"field limit"}))["project"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let d = call(
                    "create_dataset",
                    json!({"project":p,"name":"points","geometry_type":"point"}),
                )["dataset"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let original = (0..count)
                    .map(|i| (format!("k{i:03}"), json!(i)))
                    .collect::<Map<_, _>>();
                let feature = |properties: Map<String, Value>| json!({"type":"Feature","id":"a","properties":properties,"geometry":null});
                let base = call("create_workspace", json!({"project":p}))["workspace"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                call(
                    "save",
                    json!({"project":p,"workspace":base,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature(original.clone())}]}),
                );
                call(
                    "publish",
                    json!({"project":p,"workspace":base,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"base"}),
                );
                let w = call("create_workspace", json!({"project":p}))["workspace"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let mut projected = original.clone();
                projected.remove("k000");
                projected.insert("fresh".into(), Value::Null);
                let mut save = json!({"project":p,"workspace":w,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"drop":["k000"],"add":[{"name":"fresh","type":"integer"}]}]});
                if with_feature {
                    save["edits"] = json!([{"dataset":d,"feature_id":"a","feature":feature(projected.clone())}]);
                }
                call("save", save);
                assert_eq!(
                    call(
                        "features",
                        json!({"project":p,"dataset":d,"workspace":w,"feature_id":"a"})
                    )["properties"],
                    json!(projected)
                );
                if with_feature {
                    let mut t = app.storage.begin(true, Duration::from_secs(30)).unwrap();
                    let raw = t
                        .edit_base(&p, &w, &d, "a", 1)
                        .unwrap()
                        .get::<_, String>(3usize)
                        .unwrap();
                    assert_eq!(
                        crate::codec::stored::<Map<String, Value>>(&raw)
                            .unwrap()
                            .len(),
                        count + 1
                    );
                    t.commit().unwrap();
                }
                call(
                    "publish",
                    json!({"project":p,"workspace":w,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"replace one field"}),
                );
                let actual = call(
                    "features",
                    json!({"project":p,"dataset":d,"feature_id":"a"}),
                )["properties"]
                    .clone();
                assert_eq!(actual, json!(projected));
                assert_eq!(actual.as_object().unwrap().len(), count);
                assert_eq!(
                    call(
                        "features",
                        json!({"project":p,"dataset":d,"revision":1,"feature_id":"a"})
                    )["properties"],
                    json!(original)
                );
                outcomes.push(actual);
            }
            assert_eq!(outcomes[0], outcomes[1]);
        }
    }

    #[test]
    fn schema_field_limit_failure_preserves_hidden_values_and_original_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("field-limit-rollback.db");
        let app = Application::new(Storage::Sqlite(path.clone()));
        app.migrate().unwrap();
        let call = |op: &str, input: Value| app.execute("alice", op, input).unwrap();
        let export = || {
            let mut out = Vec::new();
            app.export_data(&mut out).unwrap();
            out
        };
        let p = call("create_project", json!({"name":"field rollback"}))["project"]
            .as_str()
            .unwrap()
            .to_owned();
        let d = call(
            "create_dataset",
            json!({"project":p,"name":"points","geometry_type":"point"}),
        )["dataset"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut original = (0..256)
            .map(|i| (format!("k{i:03}"), json!(i)))
            .collect::<Map<_, _>>();
        let feature = |properties: Map<String, Value>| json!({"type":"Feature","id":"a","properties":properties,"geometry":null});
        let base = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":base,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature(original.clone())}]}),
        );
        call(
            "publish",
            json!({"project":p,"workspace":base,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"base"}),
        );
        let w = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        original.insert("k000".into(), json!(999));
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":0,"edits":[{"dataset":d,"feature_id":"a","feature":feature(original.clone())}]}),
        );
        let mut projected = original.clone();
        projected.remove("k000");
        projected.insert("fresh".into(), json!(7));
        call(
            "save",
            json!({"project":p,"workspace":w,"expected_workspace_version":1,"schema_edits":[{"dataset":d,"drop":["k000"],"add":[{"name":"fresh","type":"integer"}]}],"edits":[{"dataset":d,"feature_id":"a","feature":feature(projected.clone())}]}),
        );
        let before = export();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER limit_receipt_failure BEFORE INSERT ON gl_idempotency BEGIN SELECT RAISE(ABORT,'field-limit rollback fixture'); END;").unwrap();
        let request = json!({"project":p,"workspace":w,"expected_workspace_version":2,"request_id":Uuid::new_v4(),"message":"field limit rollback"});
        let error = app
            .execute("alice", "publish", request.clone())
            .unwrap_err();
        assert_eq!(
            error.status, 503,
            "a legal 256-field candidate must reach the injected receipt failure"
        );
        assert_eq!(export(), before);
        db.execute_batch("DROP TRIGGER limit_receipt_failure")
            .unwrap();
        let recovered = Application::new(Storage::Sqlite(dir.path().join("undo.db")));
        recovered.migrate().unwrap();
        recovered
            .import_data(&mut std::io::Cursor::new(before))
            .unwrap();
        recovered
            .execute(
                "alice",
                "save",
                json!({"project":p,"workspace":w,"expected_workspace_version":2,"schema_edits":[]}),
            )
            .unwrap();
        assert_eq!(
            recovered
                .execute(
                    "alice",
                    "features",
                    json!({"project":p,"dataset":d,"workspace":w,"feature_id":"a"})
                )
                .unwrap()["properties"],
            json!(original)
        );
        let receipt = call("publish", request.clone());
        assert_eq!(receipt, call("publish", request));
        assert_eq!(
            call(
                "features",
                json!({"project":p,"dataset":d,"feature_id":"a"})
            )["properties"],
            json!(projected)
        );
        // A new pending drop must not hide an already-retired property from a
        // malformed persisted draft. Only k001 is authorized to become hidden.
        let next = call("create_workspace", json!({"project":p}))["workspace"]
            .as_str()
            .unwrap()
            .to_owned();
        call(
            "save",
            json!({"project":p,"workspace":next,"expected_workspace_version":0,"schema_edits":[{"dataset":d,"drop":["k001"],"add":[{"name":"last","type":"integer"}]}]}),
        );
        let mut raw = projected.clone();
        raw.remove("k002");
        raw.insert("k000".into(), json!(111));
        raw.insert("last".into(), Value::Null);
        let mut transaction = app.storage.begin(false, Duration::from_secs(30)).unwrap();
        transaction
            .put_deltas(
                &p,
                &next,
                &[repository::DraftDelta {
                    dataset: d.clone(),
                    key: "a".into(),
                    properties: Some(json!(raw).to_string()),
                    geometry: None,
                }],
            )
            .unwrap();
        transaction.commit().unwrap();
        let before = export();
        let invalid = app.execute("alice","publish",json!({"project":p,"workspace":next,"expected_workspace_version":1,"request_id":Uuid::new_v4(),"message":"retired is not a hidden field"})).unwrap_err();
        assert_eq!(invalid.status, 400);
        assert_eq!(export(), before);
    }
}
