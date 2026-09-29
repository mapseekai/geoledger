//! The adapter must accept merged records without silently changing their values.
use super::*;

pub(super) fn validate_merge_records(
    repo: &dyn ObjectStore,
    state: &RepositoryState,
    before: &Snapshot,
    after: &Snapshot,
    session: &mut dyn WorkingCopyTransaction,
) -> Result<()> {
    for (name, dataset) in after {
        let mut binding = state
            .bindings
            .get(name)
            .ok_or_else(|| Error::Unsupported(format!("no working-copy binding for {name}")))?
            .clone();
        binding.schema = schema::read(repo, &dataset.schema)?;
        let mut batch = Vec::new();
        let mut bytes = 0;
        let mut check = |batch: &mut Vec<Record>| -> Result<()> {
            let normalized = session.normalize_many(&binding, batch)?;
            if normalized.len() != batch.len() {
                return Err(Error::Storage("normalization row count mismatch".into()));
            }
            for (original, normalized) in batch.iter().zip(normalized) {
                if *original != normalized {
                    return Err(Error::Conflict(format!(
                        "merge would change values under the target schema in {name}, key {}; align the branch values/types before merging",
                        original.key
                    )));
                }
            }
            batch.clear();
            Ok(())
        };
        let mut visit = |id: &ObjectId| -> Result<()> {
            let record: Record = load(repo, "record/v3", id)?;
            bytes += record.payload_bytes();
            batch.push(record);
            if bytes >= 8 * 1024 * 1024 || batch.len() >= 1000 {
                check(&mut batch)?;
                bytes = 0;
            }
            Ok(())
        };
        if before.get(name).is_some_and(|d| d.schema == dataset.schema) {
            tree::visit_diff(
                repo,
                before.get(name).and_then(|d| d.root.as_ref()),
                dataset.root.as_ref(),
                &mut |d| {
                    if let Some(id) = d.after {
                        visit(&id)?;
                    }
                    Ok(())
                },
            )?;
        } else {
            tree::visit(repo, dataset.root.as_ref(), &mut |_, id| visit(id))?;
        }
        check(&mut batch)?;
    }
    Ok(())
}
