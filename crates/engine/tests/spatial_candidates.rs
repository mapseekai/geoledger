#![allow(clippy::unwrap_used)]
use geoledger_engine::{Application, Storage};
use serde_json::{Value, json};

#[test]
fn bbox_candidates_preserve_history_draft_shadowing_dimensions_and_pagination()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let app = Application::new(Storage::Sqlite(dir.path().join("spatial.db")));
    app.migrate()?;
    let project =
        app.execute("alice", "create_project", json!({"name":"spatial"}))?["project"].clone();
    let call = |op: &str, mut args: Value| {
        args["project"] = project.clone();
        app.execute("alice", op, args)
    };
    for dimension in [2, 3] {
        let dataset = call("create_dataset", json!({"name":format!("points-{dimension}"),"geometry_type":"point","coordinate_dimension":dimension}))?["dataset"].clone();
        let edit = |id: &str, x: f64| {
            let coords = if dimension == 3 {
                json!([x, x, 100.])
            } else {
                json!([x, x])
            };
            json!({"dataset":dataset,"feature_id":id,"feature":{"type":"Feature","id":id,"properties":{},"geometry":{"type":"Point","coordinates":coords}}})
        };
        let seed = call("create_workspace", json!({}))?["workspace"].clone();
        let edits: Vec<_> = (0..70).map(|n| edit(&format!("{n:03}"), 0.)).collect();
        call(
            "save",
            json!({"workspace":seed,"expected_workspace_version":0,"edits":edits}),
        )?;
        let revision = call("publish", json!({"workspace":seed,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"seed"}))?["revision"].clone();
        let draft = call("create_workspace", json!({}))?["workspace"].clone();
        call(
            "save",
            json!({"workspace":draft,"expected_workspace_version":0,"edits":[edit("000",20.),{"dataset":dataset,"feature_id":"001","feature":null},edit("070",0.)]}),
        )?;
        let page = |workspace: Value, at: Value, bbox: Value, after: &str, limit: usize| {
            call(
                "features",
                json!({"dataset":dataset,"workspace":workspace,"revision":at,"bbox":bbox,"after":after,"limit":limit}),
            )
        };
        let bounds = json!([-1., -1., 1., 1.]);
        let mut ids = Vec::new();
        let mut after = String::new();
        loop {
            let result = page(draft.clone(), Value::Null, bounds.clone(), &after, 17)?;
            let features = result["features"].as_array().unwrap();
            if features.is_empty() {
                break;
            }
            for f in features {
                ids.push(f["id"].as_str().unwrap().to_owned());
            }
            after = ids.last().unwrap().clone();
        }
        assert_eq!(ids, (2..=70).map(|n| format!("{n:03}")).collect::<Vec<_>>());
        assert_eq!(
            page(
                draft.clone(),
                Value::Null,
                json!([19., 19., 21., 21.]),
                "",
                100
            )?["features"][0]["id"],
            "000"
        );
        assert_eq!(
            page(
                draft.clone(),
                Value::Null,
                json!([-180., -90., 180., 90.]),
                "",
                100
            )?["features"]
                .as_array()
                .unwrap()
                .len(),
            70
        );
        assert!(
            page(
                draft.clone(),
                Value::Null,
                json!([40., 40., 41., 41.]),
                "",
                100
            )?["features"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        call(
            "publish",
            json!({"workspace":draft,"expected_workspace_version":1,"request_id":uuid::Uuid::new_v4(),"message":"move/delete/add"}),
        )?;
        let historical = page(Value::Null, revision, bounds.clone(), "", 100)?;
        assert_eq!(historical["features"].as_array().unwrap().len(), 70);
        assert_eq!(historical["features"][0]["id"], "000");
        let current = page(Value::Null, Value::Null, bounds, "", 100)?;
        let current_ids: Vec<_> = current["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(current_ids, ids);
    }
    Ok(())
}
