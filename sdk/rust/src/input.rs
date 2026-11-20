//! Validate business JSON before generated proto3 defaults can erase caller intent.
use crate::Error;
use serde_json::{Map, Value};

fn invalid() -> Error {
    Error::invalid("invalid operation request shape")
}

fn object<'a>(
    value: &'a Value,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a Map<String, Value>, Error> {
    let object = value.as_object().ok_or_else(invalid)?;
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(invalid());
    }
    Ok(object)
}

fn string(value: &Value) -> Result<(), Error> {
    value.as_str().map(|_| ()).ok_or_else(invalid)
}

fn edits(value: &Value) -> Result<(), Error> {
    for edit in value.as_array().ok_or_else(invalid)? {
        let edit = object(edit, &["dataset", "feature_id", "feature"], &[])?;
        string(&edit["dataset"])?;
        string(&edit["feature_id"])?;
        let feature = &edit["feature"];
        if feature.is_null() {
            continue;
        }
        let feature = object(feature, &["type", "id", "properties", "geometry"], &[])?;
        string(&feature["type"])?;
        string(&feature["id"])?;
        // Property keys and their nested JSON values are caller data, not request fields.
        if !feature["properties"].is_object()
            || !(feature["geometry"].is_null() || feature["geometry"].is_object())
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn schema_edits(value: &Value) -> Result<(), Error> {
    for edit in value.as_array().ok_or_else(invalid)? {
        let edit = object(edit, &["dataset"], &["add", "drop"])?;
        string(&edit["dataset"])?;
        if let Some(add) = edit.get("add") {
            for field in add.as_array().ok_or_else(invalid)? {
                let field = object(field, &["name", "type"], &[])?;
                string(&field["name"])?;
                string(&field["type"])?;
            }
        }
        if let Some(drop) = edit.get("drop") {
            for field in drop.as_array().ok_or_else(invalid)? {
                string(field)?;
            }
        }
    }
    Ok(())
}

pub(super) fn validate(operation: &str, value: &Value) -> Result<(), Error> {
    let (required, optional): (&[&str], &[&str]) = match operation {
        "info" => (&[], &[]),
        "create_project" => (&["name"], &[]),
        "list_projects" => (&[], &["after", "limit"]),
        "get_project" | "create_workspace" => (&["project"], &[]),
        "rename_project" => (&["project", "name"], &[]),
        "set_member" => (&["project", "subject", "role"], &[]),
        "remove_member" => (&["project", "subject"], &[]),
        "archive_project" => (&["project", "archived"], &[]),
        "delete_project" => (&["project", "confirm_name"], &[]),
        "create_dataset" => (
            &["project", "name"],
            &[
                "geometry_type",
                "coordinate_dimension",
                "crs",
                "postgis_table",
            ],
        ),
        "rename_dataset" => (&["project", "dataset", "name"], &[]),
        "delete_dataset" => (&["project", "dataset", "confirm_name"], &[]),
        "list_members" | "list_datasets" | "list_workspaces" => (&["project"], &["after", "limit"]),
        "get_workspace" | "workspace_summary" => (&["project", "workspace"], &[]),
        "save" => (
            &["project", "workspace", "expected_workspace_version"],
            &["edits", "schema_edits"],
        ),
        "discard" => (&["project", "workspace", "expected_workspace_version"], &[]),
        "features" => (
            &["project", "dataset"],
            &[
                "workspace",
                "revision",
                "feature_id",
                "bbox",
                "after",
                "limit",
            ],
        ),
        "schema" => (&["project", "dataset"], &["workspace", "revision"]),
        "history" | "audit" => (&["project"], &["after", "limit"]),
        "commit" => (&["project", "revision"], &["after", "limit"]),
        "commit_summary" | "restore" => (&["project", "revision"], &[]),
        "diff" | "conflicts" => (&["project", "workspace"], &["after", "limit"]),
        "publish" => (
            &[
                "project",
                "workspace",
                "expected_workspace_version",
                "request_id",
                "message",
            ],
            &[],
        ),
        "resolve" | "rebase" => (
            &[
                "project",
                "workspace",
                "expected_workspace_version",
                "expected_head",
                "resolutions",
            ],
            &[],
        ),
        _ => return Err(Error::invalid("unknown operation")),
    };
    let request = object(value, required, optional)?;
    for (key, value) in request {
        // Preserve presence for nullable business options, including schema intent.
        if value.is_null()
            && optional.contains(&key.as_str())
            && matches!(
                key.as_str(),
                "limit"
                    | "workspace"
                    | "revision"
                    | "feature_id"
                    | "postgis_table"
                    | "schema_edits"
            )
        {
            continue;
        }
        match key.as_str() {
            "expected_workspace_version" | "expected_head" | "revision" | "limit" => {
                value.as_i64().ok_or_else(invalid)?;
            }
            "after" if matches!(operation, "history" | "audit") => {
                value.as_i64().ok_or_else(invalid)?;
            }
            "coordinate_dimension" => {
                let number = value.as_u64().ok_or_else(invalid)?;
                u32::try_from(number).map_err(|_| invalid())?;
            }
            "archived" => {
                value.as_bool().ok_or_else(invalid)?;
            }
            "bbox" => {
                for coordinate in value.as_array().ok_or_else(invalid)? {
                    if !coordinate.as_f64().is_some_and(f64::is_finite) {
                        return Err(invalid());
                    }
                }
            }
            "edits" | "resolutions" => edits(value)?,
            "schema_edits" => schema_edits(value)?,
            "postgis_table" => {
                let table = object(
                    value,
                    &["schema", "table", "id_column", "geometry_column"],
                    &[],
                )?;
                for value in table.values() {
                    string(value)?;
                }
            }
            _ => string(value)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_operation_rejects_unknown_missing_and_wrongly_typed_required_fields()
    -> Result<(), Error> {
        let requests = [
            ("info", json!({})),
            ("create_project", json!({"name":"roads"})),
            ("list_projects", json!({})),
            ("get_project", json!({"project":"p"})),
            ("create_workspace", json!({"project":"p"})),
            ("rename_project", json!({"project":"p","name":"roads"})),
            (
                "set_member",
                json!({"project":"p","subject":"s","role":"viewer"}),
            ),
            ("remove_member", json!({"project":"p","subject":"s"})),
            ("archive_project", json!({"project":"p","archived":false})),
            (
                "delete_project",
                json!({"project":"p","confirm_name":"roads"}),
            ),
            ("create_dataset", json!({"project":"p","name":"points"})),
            (
                "rename_dataset",
                json!({"project":"p","dataset":"d","name":"points"}),
            ),
            (
                "delete_dataset",
                json!({"project":"p","dataset":"d","confirm_name":"points"}),
            ),
            ("list_members", json!({"project":"p"})),
            ("list_datasets", json!({"project":"p"})),
            ("list_workspaces", json!({"project":"p"})),
            ("get_workspace", json!({"project":"p","workspace":"w"})),
            ("workspace_summary", json!({"project":"p","workspace":"w"})),
            (
                "save",
                json!({"project":"p","workspace":"w","expected_workspace_version":0}),
            ),
            (
                "discard",
                json!({"project":"p","workspace":"w","expected_workspace_version":0}),
            ),
            ("features", json!({"project":"p","dataset":"d"})),
            ("schema", json!({"project":"p","dataset":"d"})),
            ("history", json!({"project":"p"})),
            ("audit", json!({"project":"p"})),
            ("commit", json!({"project":"p","revision":0})),
            ("commit_summary", json!({"project":"p","revision":0})),
            ("restore", json!({"project":"p","revision":0})),
            ("diff", json!({"project":"p","workspace":"w"})),
            ("conflicts", json!({"project":"p","workspace":"w"})),
            (
                "publish",
                json!({"project":"p","workspace":"w","expected_workspace_version":0,"request_id":"r","message":""}),
            ),
            (
                "resolve",
                json!({"project":"p","workspace":"w","expected_workspace_version":0,"expected_head":0,"resolutions":[]}),
            ),
            (
                "rebase",
                json!({"project":"p","workspace":"w","expected_workspace_version":0,"expected_head":0,"resolutions":[]}),
            ),
        ];
        for (operation, request) in requests {
            validate(operation, &request)?;
            for non_object in [Value::Null, json!([]), json!("request")] {
                assert!(validate(operation, &non_object).is_err(), "{operation}");
            }
            let fields = request.as_object().ok_or_else(invalid)?;
            let mut unknown = request.clone();
            unknown["obsolete"] = json!(true);
            assert!(validate(operation, &unknown).is_err(), "{operation}");
            for field in fields.keys() {
                let mut missing = fields.clone();
                missing.remove(field);
                assert!(
                    validate(operation, &Value::Object(missing)).is_err(),
                    "{operation}.{field}"
                );
                let mut wrong_type = request.clone();
                wrong_type[field] = if fields[field].is_array() {
                    json!(false)
                } else {
                    json!([])
                };
                assert!(
                    validate(operation, &wrong_type).is_err(),
                    "{operation}.{field}"
                );
                let mut null = request.clone();
                null[field] = Value::Null;
                assert!(validate(operation, &null).is_err(), "{operation}.{field}");
            }
        }
        assert!(validate("obsolete_operation", &json!({})).is_err());
        Ok(())
    }

    #[test]
    fn current_optional_presence_defaults_and_integer_ranges_are_preserved() -> Result<(), Error> {
        validate(
            "list_projects",
            &serde_json::to_value(crate::Page::default())?,
        )?;
        let mut features = serde_json::to_value(crate::FeatureQuery::default())?;
        features["project"] = json!("p");
        features["dataset"] = json!("d");
        validate("features", &features)?;
        validate(
            "schema",
            &json!({"project":"p","dataset":"d","workspace":null,"revision":null}),
        )?;
        validate(
            "create_dataset",
            &json!({"project":"p","name":"d","postgis_table":null}),
        )?;
        for schema in [None, Some(Value::Null), Some(json!([]))] {
            let mut request =
                json!({"project":"p","workspace":"w","expected_workspace_version":0,"edits":[]});
            if let Some(schema) = schema {
                request["schema_edits"] = schema;
            }
            validate("save", &request)?;
        }
        validate(
            "history",
            &json!({"project":"p","after":9007199254740993_i64,"limit":null}),
        )?;
        for (operation, request) in [
            (
                "save",
                json!({"project":"p","workspace":"w","expected_workspace_version":"0"}),
            ),
            (
                "restore",
                json!({"project":"p","revision":18446744073709551615_u64}),
            ),
            (
                "features",
                json!({"project":"p","dataset":"d","bbox":["1",2,3,4]}),
            ),
            ("features", json!({"project":"p","dataset":"d","limit":1.5})),
            ("history", json!({"project":"p","after":"1"})),
            ("diff", json!({"project":"p","workspace":"w","after":1})),
            (
                "create_dataset",
                json!({"project":"p","name":"d","coordinate_dimension":4294967296_u64}),
            ),
        ] {
            assert!(validate(operation, &request).is_err(), "{operation}");
        }
        Ok(())
    }

    #[test]
    fn nested_control_fields_are_strict_and_feature_properties_are_opaque() -> Result<(), Error> {
        let feature = json!({"type":"Feature","id":"one","geometry":null,"properties":{
            "project":{"obsolete":true},"edits":[{"feature":null}],
            "geojson":"ordinary text","exact":18446744073709551615_u64
        }});
        let request = json!({"project":"p","workspace":"w","expected_workspace_version":0,
            "edits":[{"dataset":"d","feature_id":"one","feature":feature},
                     {"dataset":"d","feature_id":"deleted","feature":null}],
            "schema_edits":[{"dataset":"d","add":[{"name":"title","type":"string"}],"drop":[]}]});
        let original = request.clone();
        validate("save", &request)?;
        assert_eq!(request, original);
        for invalid_edit in [
            json!({"dataset":"d","feature_id":"one"}),
            json!({"dataset":"d","feature_id":"one","feature":null,"obsolete":true}),
            json!({"dataset":"d","feature_id":"one","feature":{"geojson":"{}"}}),
        ] {
            let mut malformed = request.clone();
            malformed["edits"] = json!([invalid_edit]);
            assert!(validate("save", &malformed).is_err());
        }
        for invalid_schema in [
            json!([{"dataset":"d","obsolete":true}]),
            json!([{"dataset":"d","add":[{"name":"title"}]}]),
            json!([{"dataset":"d","add":[{"name":"title","type":"string","sql":"text"}]}]),
            json!([{"dataset":"d","drop":[1]}]),
            json!({"edits":[]}),
        ] {
            let mut malformed = request.clone();
            malformed["schema_edits"] = invalid_schema;
            assert!(validate("save", &malformed).is_err());
        }
        let table =
            json!({"schema":"business","table":"roads","id_column":"id","geometry_column":"geom"});
        validate(
            "create_dataset",
            &json!({"project":"p","name":"roads","postgis_table":table}),
        )?;
        for invalid_table in [
            json!({"schema":"business","table":"roads","id_column":"id"}),
            json!({"schema":"business","table":"roads","id_column":"id","geometry_column":"geom","obsolete":true}),
        ] {
            assert!(
                validate(
                    "create_dataset",
                    &json!({"project":"p","name":"roads","postgis_table":invalid_table})
                )
                .is_err()
            );
        }
        Ok(())
    }
}
