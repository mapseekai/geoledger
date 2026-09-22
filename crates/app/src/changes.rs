//! Bounded working-copy previews and streaming snapshot application.
use super::*;

pub(super) const PREVIEW_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub(super) struct WorkingChanges {
    preview: Vec<Change>,
    total: usize,
    preview_bytes: usize,
    pub inserted: usize,
    pub deleted: usize,
    pub snapshot: Snapshot,
}
impl WorkingChanges {
    pub fn len(&self) -> usize {
        self.total
    }
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
    pub fn page(&self, limit: usize) -> Value {
        let records: Vec<_> = self.preview.iter().take(limit.clamp(1, 1000)).collect();
        json!({"truncated":self.total>records.len(),"changes":records,"total":self.total})
    }
}

pub(super) fn working_changes(
    repo: &dyn ObjectStore,
    bindings: &BTreeMap<String, Binding>,
    schema_dirty: &BTreeMap<String, Schema>,
    baseline: &Snapshot,
    session: &mut dyn WorkingCopyTransaction,
    command: &Command,
) -> Result<WorkingChanges> {
    let limit = match command {
        Command::Status { limit } | Command::Diff { limit, .. } => (*limit).clamp(1, 1000),
        _ => 0,
    };
    let capture = matches!(
        command,
        Command::Commit { .. }
            | Command::Diff {
                from: Some(_),
                to: None,
                ..
            }
    );
    let restore = matches!(
        command,
        Command::Restore { discard: true } | Command::Reset { hard: true, .. }
    );
    let mut summary = WorkingChanges {
        snapshot: baseline.clone(),
        ..Default::default()
    };
    for (dataset, binding) in bindings
        .iter()
        .filter(|(name, _)| !schema_dirty.contains_key(*name))
    {
        let mut cursor = None;
        loop {
            let keys = session.dirty_keys_page(dataset, cursor.as_deref(), 1000)?;
            if keys.is_empty() {
                break;
            }
            let mut offset = 0;
            while offset < keys.len() {
                let records =
                    session.read_many_bounded(binding, &keys[offset..], 8 * 1024 * 1024)?;
                if records.is_empty() || records.len() > keys.len() - offset {
                    return Err(Error::Database("bounded read made no progress".into()));
                }
                if records
                    .iter()
                    .zip(&keys[offset..])
                    .any(|((key, _), expected)| key != expected)
                {
                    return Err(Error::Database(
                        "bounded read returned out-of-order keys".into(),
                    ));
                }
                offset += records.len();
                let mut reverse = Vec::new();
                let mut bytes = 0;
                for (key, after) in records {
                    let before = merge::record(repo, baseline.get(dataset), &key)?;
                    if before == after {
                        continue;
                    }
                    summary.total += 1;
                    summary.inserted += usize::from(before.is_none());
                    summary.deleted += usize::from(after.is_none());
                    if capture {
                        let target = summary.snapshot.get_mut(dataset).ok_or_else(|| {
                            Error::Storage("missing dataset for working-copy capture".into())
                        })?;
                        merge::update(repo, target, &key, after.as_ref())?;
                    }
                    if summary.preview.len() < limit && summary.preview_bytes < PREVIEW_BYTES {
                        summary.preview_bytes += before.as_ref().map_or(0, record_size)
                            + after.as_ref().map_or(0, record_size);
                        summary.preview.push(Change {
                            dataset: dataset.clone(),
                            key: key.clone(),
                            fields: merge::changed_fields(before.as_ref(), after.as_ref()),
                            before: before.clone(),
                            after,
                        });
                    }
                    if restore {
                        bytes += before.as_ref().map_or(0, record_size);
                        reverse.push((key.clone(), before));
                        if bytes >= 8 * 1024 * 1024 || reverse.len() >= 1000 {
                            flush(session, binding, &mut reverse)?;
                            bytes = 0;
                        }
                    }
                }
                flush(session, binding, &mut reverse)?;
            }
            cursor = keys.last().cloned();
        }
    }
    Ok(summary)
}

fn flush(
    session: &mut dyn WorkingCopyTransaction,
    binding: &Binding,
    batch: &mut Vec<(String, Option<Record>)>,
) -> Result<()> {
    if !batch.is_empty() {
        session.write_many(
            binding,
            &batch
                .iter()
                .map(|(key, record)| (key.as_str(), record.as_ref()))
                .collect::<Vec<_>>(),
        )?;
        batch.clear();
    }
    Ok(())
}

pub(super) fn write_snapshot_diff(
    repo: &dyn ObjectStore,
    state: &RepositoryState,
    before: &Snapshot,
    after: &Snapshot,
    session: &mut dyn WorkingCopyTransaction,
) -> Result<()> {
    for (name, binding) in &state.bindings {
        let a = before
            .get(name)
            .map(|d| (name.clone(), d.clone()))
            .into_iter()
            .collect();
        let b = after
            .get(name)
            .map(|d| (name.clone(), d.clone()))
            .into_iter()
            .collect();
        let mut batch = Vec::new();
        let mut bytes = 0;
        merge::visit_diff(repo, &a, &b, &mut |change| {
            bytes += change.after.as_ref().map_or(0, record_size);
            batch.push((change.key, change.after));
            if bytes >= 8 * 1024 * 1024 || batch.len() >= 1000 {
                flush(session, binding, &mut batch)?;
                bytes = 0;
            }
            Ok(())
        })?;
        flush(session, binding, &mut batch)?;
    }
    Ok(())
}
