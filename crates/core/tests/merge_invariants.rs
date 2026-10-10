//! Small exhaustive state space also runs under Miri without database/native dependencies.
use geoledger_core::{Cell, Record, merge::merge_record};
use std::collections::BTreeMap;

#[test]
fn merge_identity_symmetry_and_independent_ownership() {
    let mut states = vec![None];
    for value in [
        None,
        Some(Cell::Null),
        Some(Cell::Text("旧值🗺".into())),
        Some(Cell::Text("new".into())),
    ] {
        states.push(Some(Record {
            key: "feature".into(),
            fields: value
                .map(|v| [("field".into(), v)].into())
                .unwrap_or_default(),
        }));
    }
    for base in &states {
        for ours in &states {
            for theirs in &states {
                let merged = merge_record(base.as_ref(), ours.as_ref(), theirs.as_ref());
                assert_eq!(
                    merged,
                    merge_record(base.as_ref(), theirs.as_ref(), ours.as_ref())
                );
                if ours == theirs {
                    assert_eq!(merged, Ok(ours.clone()));
                }
                if ours == base {
                    assert_eq!(merged, Ok(theirs.clone()));
                }
                if theirs == base {
                    assert_eq!(merged, Ok(ours.clone()));
                }
            }
        }
    }
    let base = Record {
        key: "feature".into(),
        fields: BTreeMap::new(),
    };
    let mut ours = base.clone();
    ours.fields.insert("left".into(), Cell::Text("a".into()));
    let mut theirs = base.clone();
    theirs.fields.insert("right".into(), Cell::Text("b".into()));
    let merged = merge_record(Some(&base), Some(&ours), Some(&theirs));
    drop(ours);
    drop(theirs);
    drop(base);
    assert_eq!(
        merged,
        Ok(Some(Record {
            key: "feature".into(),
            fields: [
                ("left".into(), Cell::Text("a".into())),
                ("right".into(), Cell::Text("b".into()))
            ]
            .into()
        }))
    );
}
