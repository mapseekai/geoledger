use crate::Record;
use std::collections::BTreeSet;

/// Three-way merge of versions of the same record key.
///
/// `None` represents absence/deletion; `Ok(None)` is a merged deletion.
/// Conflicts return sorted field names, or `"*"` for incompatible record-level
/// insertion/deletion. Field values are compared exactly, without JSON normalization.
pub fn merge_record(
    base: Option<&Record>,
    ours: Option<&Record>,
    theirs: Option<&Record>,
) -> std::result::Result<Option<Record>, Vec<String>> {
    if ours == theirs {
        return Ok(ours.cloned());
    }
    if ours == base {
        return Ok(theirs.cloned());
    }
    if theirs == base {
        return Ok(ours.cloned());
    }
    let (Some(base), Some(ours), Some(theirs)) = (base, ours, theirs) else {
        return Err(vec!["*".into()]);
    };
    let keys: BTreeSet<_> = base
        .fields
        .keys()
        .chain(ours.fields.keys())
        .chain(theirs.fields.keys())
        .collect();
    let mut selected_fields = Vec::new();
    let mut conflicts = Vec::new();
    for k in keys {
        let (b, o, t) = (base.fields.get(k), ours.fields.get(k), theirs.fields.get(k));
        let selected = if o == t || t == b {
            o
        } else if o == b {
            t
        } else {
            conflicts.push(k.clone());
            continue;
        };
        if let Some(value) = selected {
            selected_fields.push((k, value));
        }
    }
    if conflicts.is_empty() {
        Ok(Some(Record {
            key: ours.key.clone(),
            fields: selected_fields
                .into_iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }))
    } else {
        Err(conflicts)
    }
}

#[cfg(test)]
mod tests {
    //! Small exhaustive state space also runs under Miri without database/native dependencies.
    use super::merge_record;
    use crate::{Cell, Record};
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
}
