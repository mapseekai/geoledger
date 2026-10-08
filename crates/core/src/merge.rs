use crate::Record;
use std::collections::BTreeSet;

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
