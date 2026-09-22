//! Application service. Synchronous by design: callers in an async runtime must
//! use spawn_blocking (the provided HTTP/gRPC server and CLI do so).
mod changes;
mod command;
mod merge_working;
use changes::*;
pub use command::{Command, Resolution};
use merge_working::*;
pub use spatial_version_core as core;

use core::{adapter::*, graph, merge, tree, *};
use serde_json::{Value, json};
use spatial_version_storage::Repository;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct Application {
    root: PathBuf,
    provider: Option<Arc<dyn WorkingCopyProvider>>,
}
impl Application {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            root: path.into(),
            provider: None,
        }
    }
    pub fn with_provider(mut self, provider: Arc<dyn WorkingCopyProvider>) -> Self {
        self.provider = Some(provider);
        self
    }
    pub fn repository_path(&self) -> &Path {
        &self.root
    }
    fn provider(&self) -> Result<&dyn WorkingCopyProvider> {
        self.provider.as_deref().ok_or_else(|| {
            Error::Invalid("configure a working-copy provider (CLI: SV_DATABASE_URL)".into())
        })
    }
    pub fn execute(&self, command: Command) -> Result<Value> {
        if let Command::Init { author } = command {
            return self.init(&author);
        }
        let repo = Repository::open(&self.root)?;
        let recovered = self.recover_pending(&repo)?;
        let mut state = repo.state()?;
        match command {
            Command::Schema { dataset, reference } => {
                let snapshot = snapshot_at(&repo, &state.resolve(&reference)?)?;
                let d = snapshot
                    .get(&dataset)
                    .ok_or_else(|| Error::NotFound(format!("dataset {dataset}")))?;
                Ok(json!({"dataset":dataset,"schema":schema::read(&repo,&d.schema)?}))
            }
            Command::Recover => Ok(json!({"recovered":recovered,"head":state.head()?})),
            Command::Log { reference, limit } => {
                let id = state.resolve(&reference)?;
                let entries = graph::log(&repo, &id, limit)?
                    .into_iter()
                    .map(|(id, commit)| json!({"id":id,"commit":commit}))
                    .collect::<Vec<_>>();
                Ok(json!({"commits":entries}))
            }
            Command::Show {
                reference,
                dataset,
                key,
            } => {
                let id = state.resolve(&reference)?;
                let commit = graph::commit(&repo, &id)?;
                let snapshot: Snapshot = load(&repo, "snapshot/v1", &commit.root)?;
                match (dataset, key) {
                    (Some(dataset), Some(key)) => Ok(
                        json!({"commit":id,"dataset":dataset,"record":merge::record(&repo,snapshot.get(&dataset),&key)?}),
                    ),
                    (None, None) => Ok(json!({"id":id,"commit":commit,"datasets":snapshot})),
                    _ => Err(Error::Invalid(
                        "dataset and key must be provided together".into(),
                    )),
                }
            }
            Command::Branches => Ok(json!({"current":state.branch,"branches":state.branches})),
            Command::Branch { name, from } => {
                ensure_not_merging(&state)?;
                validate_branch(&name)?;
                if state.branches.contains_key(&name) {
                    return Err(Error::Conflict("branch already exists".into()));
                }
                let id = state.resolve(&from)?;
                graph::commit(&repo, &id)?;
                state.branches.insert(name.clone(), id.clone());
                repo.begin()?;
                repo.save_state(&state)?;
                repo.commit()?;
                Ok(json!({"branch":name,"head":id}))
            }
            Command::Conflicts { limit } => {
                let conflicts = state
                    .merging
                    .as_ref()
                    .map(|m| m.conflicts.as_slice())
                    .unwrap_or(&[]);
                Ok(page("conflicts", conflicts, limit))
            }
            Command::MergeAbort => {
                if state.merging.is_none() {
                    return Err(Error::Conflict("no merge or revert in progress".into()));
                }
                state.merging = None;
                repo.begin()?;
                repo.save_state(&state)?;
                repo.commit()?;
                Ok(json!({"aborted":true,"head":state.head()?,"working_copy_changed":false}))
            }
            Command::Reflog { limit } => Ok(json!({"entries":repo.reflog(limit)?})),
            Command::Fsck => fsck(&repo, &state),
            Command::Diff {
                from,
                to: Some(to),
                limit,
            } => {
                let a = snapshot_at(&repo, &state.resolve(from.as_deref().unwrap_or("HEAD"))?)?;
                let b = snapshot_at(&repo, &state.resolve(&to)?)?;
                let limit = limit.clamp(1, 1000);
                let (changes, total) = merge::diff_page(&repo, &a, &b, limit, PREVIEW_BYTES)?;
                let mut result =
                    json!({"changes":changes,"total":total,"truncated":total>changes.len()});
                result["schema_changes"] = schema_changes(&repo, &a, &b)?;
                Ok(result)
            }
            Command::Status { limit } if state.bindings.is_empty() => {
                Ok(status(&state, &WorkingChanges::default(), limit))
            }
            other => self.execute_working(&repo, state, other),
        }
    }
    fn init(&self, author: &str) -> Result<Value> {
        identity(author, "Initialize spatial repository")?;
        let repo = Repository::init(&self.root)?;
        repo.begin()?;
        let initial = create_commit(
            &repo,
            &Snapshot::new(),
            Vec::new(),
            author,
            "Initialize spatial repository",
        )?;
        let state = RepositoryState {
            version: 1,
            repository_id: uuid::Uuid::new_v4().to_string(),
            branch: "main".into(),
            branches: BTreeMap::from([("main".into(), initial.clone())]),
            bindings: BTreeMap::new(),
            merging: None,
        };
        repo.save_state(&state)?;
        repo.commit()?;
        Ok(
            json!({"repository":self.root,"repository_id":state.repository_id,"head":initial,"branch":"main","format_version":1}),
        )
    }
    fn recover_pending(&self, repo: &Repository) -> Result<bool> {
        let Some(pending) = repo.pending()? else {
            return Ok(false);
        };
        let before = repo.state()?;
        let mut session = self.provider()?.begin(
            &before.repository_id,
            before.head()?,
            &before.bindings,
            None,
        )?;
        let marker = session.marker()?;
        let applied = marker.operation.as_deref() == Some(pending.id.as_str());
        if applied {
            if marker.head != pending.after.head()?.as_str() {
                return Err(Error::Recovery(
                    "database marker disagrees with prepared result".into(),
                ));
            }
        } else if marker.head != pending.before_head.as_str() {
            return Err(Error::Recovery(
                "database has advanced outside the pending operation".into(),
            ));
        }
        session.commit()?;
        repo.begin()?;
        if applied {
            repo.save_state(&pending.after)?;
        }
        repo.clear_pending()?;
        repo.commit()?;
        Ok(true)
    }
    fn execute_working(
        &self,
        repo: &Repository,
        mut state: RepositoryState,
        command: Command,
    ) -> Result<Value> {
        if !matches!(
            command,
            Command::Status { .. }
                | Command::Diff { .. }
                | Command::Resolve { .. }
                | Command::MergeContinue
        ) {
            ensure_not_merging(&state)?;
        }
        let extra = match &command {
            Command::Import { schema, table, .. } => Some((schema.as_str(), table.as_str())),
            _ => None,
        };
        let provider = self.provider()?;
        if state
            .bindings
            .values()
            .any(|b| b.provider != provider.name())
        {
            return Err(Error::Unsupported(
                "mixed working-copy providers in one repository".into(),
            ));
        }
        let mut session =
            provider.begin(&state.repository_id, state.head()?, &state.bindings, extra)?;
        let marker = session.marker()?;
        if marker.head != state.head()?.as_str() {
            return Err(Error::Recovery(
                "working-copy HEAD differs from repository HEAD; refusing overwrite".into(),
            ));
        }
        for (dataset, binding) in &state.bindings {
            session.verify(dataset, binding)?;
        }
        repo.begin()?;
        let before_head = state.head()?.clone();
        let baseline = snapshot_at(repo, &before_head)?;
        let mut schema_dirty = BTreeMap::new();
        for (name, binding) in &state.bindings {
            if binding.schema.version == 2 {
                let actual = session.current_schema(binding)?;
                if actual != binding.schema {
                    schema_dirty.insert(name.clone(), actual);
                }
            }
        }
        if !schema_dirty.is_empty()
            && !matches!(
                command,
                Command::Status { .. }
                    | Command::Diff { .. }
                    | Command::Commit { .. }
                    | Command::Restore { .. }
                    | Command::Reset { .. }
            )
        {
            return Err(Error::Dirty);
        }
        let changes = working_changes(
            repo,
            &state.bindings,
            &schema_dirty,
            &baseline,
            session.as_mut(),
            &command,
        )?;
        match command {
            Command::Status { limit } => {
                let mut result = status(&state, &changes, limit);
                result["clean"] = json!(changes.is_empty() && schema_dirty.is_empty());
                result["record_counts_complete"] = json!(schema_dirty.is_empty());
                result["schema_changes"]=json!(schema_dirty.iter().map(|(name,after)|json!({"dataset":name,"before":state.bindings[name].schema,"after":after,"requires_full_scan":true})).collect::<Vec<_>>());
                Ok(result)
            }
            Command::Diff {
                from,
                to: None,
                limit,
            } => {
                if !schema_dirty.is_empty() {
                    let mut result = changes.page(limit);
                    result["record_counts_complete"] = json!(false);
                    result["schema_changes"]=json!(schema_dirty.iter().map(|(name,after)|json!({"dataset":name,"before":state.bindings[name].schema,"after":after,"requires_full_scan":true})).collect::<Vec<_>>());
                    if from.is_some() {
                        return Err(Error::Unsupported(
                            "historical diff with uncommitted schema changes; commit first".into(),
                        ));
                    }
                    return Ok(result);
                }
                if let Some(reference) = from {
                    let source = snapshot_at(repo, &state.resolve(&reference)?)?;
                    let actual = changes.snapshot.clone();
                    let limit = limit.clamp(1, 1000);
                    let (changes, total) =
                        merge::diff_page(repo, &source, &actual, limit, PREVIEW_BYTES)?;
                    Ok(json!({"changes":changes,"total":total,"truncated":total>changes.len()}))
                } else {
                    Ok(changes.page(limit))
                }
            }
            Command::Import {
                dataset,
                schema,
                table,
                author,
                message,
            } => {
                ensure_clean(&changes)?;
                if dataset.is_empty() || dataset.len() > 255 || dataset.contains('\0') {
                    return Err(Error::Invalid("invalid dataset name".into()));
                }
                if state.bindings.contains_key(&dataset) {
                    return Err(Error::Conflict("dataset is already registered".into()));
                }
                let schema_def = schema::with_identities(session.inspect(&schema, &table)?);
                let mut binding = Binding {
                    provider: provider.name().into(),
                    schema_name: schema,
                    table_name: table,
                    schema: schema_def,
                    column_ids: BTreeMap::new(),
                };
                binding.column_ids = session.column_ids(&binding)?;
                binding.schema = session.current_schema(&binding)?;
                session.register(&dataset, &binding)?;
                let mut tree = tree::BulkBuilder::default();
                let mut records = 0u64;
                session.scan(&binding, &mut |record| {
                    let id = save(repo, "record/v1", &record)?;
                    tree.push(repo, record.key, id)?;
                    records += 1;
                    Ok(())
                })?;
                let imported = Dataset {
                    schema: schema::store(repo, &binding.schema)?,
                    root: tree.finish(repo)?,
                    records,
                };
                let mut snapshot = baseline;
                snapshot.insert(dataset.clone(), imported);
                state.bindings.insert(dataset.clone(), binding);
                state.version = 2;
                let message = message.unwrap_or_else(|| format!("Import {dataset}"));
                let id = create_commit(
                    repo,
                    &snapshot,
                    vec![before_head.clone()],
                    &author,
                    &message,
                )?;
                state.branches.insert(state.branch.clone(), id.clone());
                finish(repo, session.as_mut(), &before_head, &state)?;
                Ok(json!({"commit":id,"dataset":dataset,"records":records}))
            }
            Command::Commit { message, author } => {
                if changes.is_empty() && schema_dirty.is_empty() {
                    return Err(Error::Conflict("nothing to commit".into()));
                }
                let mut snapshot = changes.snapshot.clone();
                for (name, actual) in &schema_dirty {
                    let binding = state
                        .bindings
                        .get_mut(name)
                        .ok_or_else(|| Error::Storage("missing binding".into()))?;
                    binding.schema = actual.clone();
                    binding.column_ids = session.column_ids(binding)?;
                    snapshot.insert(
                        name.clone(),
                        capture_dataset(repo, binding, session.as_mut())?,
                    );
                }
                let id = create_commit(
                    repo,
                    &snapshot,
                    vec![before_head.clone()],
                    &author,
                    &message,
                )?;
                state.branches.insert(state.branch.clone(), id.clone());
                finish(repo, session.as_mut(), &before_head, &state)?;
                Ok(
                    json!({"commit":id,"changed_records":if schema_dirty.is_empty(){Some(changes.len())}else{None},"incremental_changed_records":changes.len(),"rescanned_records":schema_dirty.keys().map(|name|snapshot[name].records).sum::<u64>(),"schema_changed_datasets":schema_dirty.keys().collect::<Vec<_>>(),"schema_datasets_rescanned":!schema_dirty.is_empty()}),
                )
            }
            Command::Upgrade => {
                ensure_clean(&changes)?;
                if state.version == 2 && state.bindings.values().all(|b| b.schema.version == 2) {
                    return Ok(
                        json!({"format_version":2,"already_current":true,"head":before_head}),
                    );
                }
                let mut snapshot = baseline.clone();
                for (name, binding) in &mut state.bindings {
                    if binding.schema.version == 1 {
                        binding.schema = schema::with_identities(binding.schema.clone());
                        binding.column_ids = session.column_ids(binding)?;
                        binding.schema = session.current_schema(binding)?;
                        if let Some(d) = snapshot.get_mut(name) {
                            d.schema = schema::store(repo, &binding.schema)?;
                        }
                    }
                }
                let id = create_commit(
                    repo,
                    &snapshot,
                    vec![before_head.clone()],
                    "migration",
                    "Upgrade schema tracking to format v2",
                )?;
                state.version = 2;
                state.branches.insert(state.branch.clone(), id.clone());
                finish(repo, session.as_mut(), &before_head, &state)?;
                Ok(json!({"commit":id,"format_version":2}))
            }
            Command::AlterSchema {
                dataset,
                change,
                author,
                message,
            } => {
                ensure_clean(&changes)?;
                let binding = state
                    .bindings
                    .get_mut(&dataset)
                    .ok_or_else(|| Error::NotFound(format!("dataset {dataset}")))?;
                if binding.schema.version != 2 {
                    return Err(Error::Unsupported(
                        "run upgrade before editing schema in a v1 dataset".into(),
                    ));
                }
                binding.schema = session.edit_schema(binding, &change)?;
                binding.column_ids = session.column_ids(binding)?;
                let mut snapshot = baseline.clone();
                snapshot.insert(
                    dataset.clone(),
                    capture_dataset(repo, binding, session.as_mut())?,
                );
                let id = create_commit(
                    repo,
                    &snapshot,
                    vec![before_head.clone()],
                    &author,
                    &message.unwrap_or_else(|| format!("Alter schema of {dataset}")),
                )?;
                state.version = 2;
                state.branches.insert(state.branch.clone(), id.clone());
                finish(repo, session.as_mut(), &before_head, &state)?;
                Ok(json!({"commit":id,"dataset":dataset,"schema":state.bindings[&dataset].schema}))
            }
            Command::Switch { branch } => {
                ensure_clean(&changes)?;
                let target = state
                    .branches
                    .get(&branch)
                    .cloned()
                    .ok_or_else(|| Error::NotFound(format!("branch {branch}")))?;
                let snapshot = snapshot_at(repo, &target)?;
                apply_snapshot(repo, &mut state, &baseline, &snapshot, session.as_mut())?;
                state.branch = branch;
                finish(repo, session.as_mut(), &before_head, &state)?;
                Ok(json!({"branch":state.branch,"head":target}))
            }
            Command::Restore { discard } => {
                if !discard {
                    return Err(Error::Invalid(
                        "restore requires explicit discard=true / --discard".into(),
                    ));
                }
                restore_schema_edits(repo, &mut state, &baseline, &schema_dirty, session.as_mut())?;
                finish(repo, session.as_mut(), &before_head, &state)?;
                Ok(json!({"restored_records":changes.len(),"head":before_head}))
            }
            Command::Reset { target, hard } => {
                if !hard {
                    return Err(Error::Unsupported(
                        "only explicit hard reset is implemented; use revert to preserve history"
                            .into(),
                    ));
                }
                let target = state.resolve(&target)?;
                let snapshot = snapshot_at(repo, &target)?;
                restore_schema_edits(repo, &mut state, &baseline, &schema_dirty, session.as_mut())?;
                apply_snapshot(repo, &mut state, &baseline, &snapshot, session.as_mut())?;
                state.branches.insert(state.branch.clone(), target.clone());
                finish(repo, session.as_mut(), &before_head, &state)?;
                Ok(
                    json!({"head":target,"previous_head":before_head,"discarded_records":changes.len()}),
                )
            }
            Command::Merge {
                source,
                author,
                message,
            } => {
                ensure_clean(&changes)?;
                let theirs = state.resolve(&source)?;
                let base = graph::merge_base(repo, &before_head, &theirs)?;
                if base == theirs {
                    return Ok(json!({"head":before_head,"up_to_date":true}));
                }
                let other = snapshot_at(repo, &theirs)?;
                if base == before_head {
                    apply_snapshot(repo, &mut state, &baseline, &other, session.as_mut())?;
                    state.branches.insert(state.branch.clone(), theirs.clone());
                    finish(repo, session.as_mut(), &before_head, &state)?;
                    return Ok(json!({"commit":theirs,"fast_forward":true}));
                }
                let base_snapshot = snapshot_at(repo, &base)?;
                let (snapshot, conflicts) = merge::three_way_with_defaults(
                    repo,
                    &base_snapshot,
                    &baseline,
                    &other,
                    &mut |from, to| session.projection_defaults(from, to),
                )?;
                let merge_state = MergeState {
                    base,
                    ours: before_head.clone(),
                    theirs: theirs.clone(),
                    parents: vec![before_head.clone(), theirs],
                    snapshot,
                    conflicts,
                    author,
                    message: message
                        .unwrap_or_else(|| format!("Merge {source} into {}", state.branch)),
                };
                complete_or_stage(repo, &mut state, &baseline, merge_state, session.as_mut())
            }
            Command::Revert {
                target,
                author,
                message,
            } => {
                ensure_clean(&changes)?;
                let target = state.resolve(&target)?;
                let commit = graph::commit(repo, &target)?;
                if commit.parents.len() != 1 {
                    return Err(Error::Unsupported(
                        "reverting root or merge commits; mainline selection is not implemented"
                            .into(),
                    ));
                }
                let parent = commit.parents[0].clone();
                let (snapshot, conflicts) = merge::three_way_with_defaults(
                    repo,
                    &snapshot_at(repo, &target)?,
                    &baseline,
                    &snapshot_at(repo, &parent)?,
                    &mut |from, to| session.projection_defaults(from, to),
                )?;
                let merge_state = MergeState {
                    base: target.clone(),
                    ours: before_head.clone(),
                    theirs: parent,
                    parents: vec![before_head.clone()],
                    snapshot,
                    conflicts,
                    author,
                    message: message.unwrap_or_else(|| format!("Revert {target}")),
                };
                complete_or_stage(repo, &mut state, &baseline, merge_state, session.as_mut())
            }
            Command::Resolve {
                dataset,
                key,
                choice,
                record,
            } => {
                ensure_clean(&changes)?;
                let binding = state
                    .bindings
                    .get(&dataset)
                    .ok_or_else(|| Error::NotFound(format!("dataset {dataset}")))?;
                let pending = state
                    .merging
                    .as_mut()
                    .ok_or_else(|| Error::Conflict("no merge in progress".into()))?;
                let index = pending
                    .conflicts
                    .iter()
                    .position(|c| c.dataset == dataset && c.key == key)
                    .ok_or_else(|| Error::NotFound("unresolved conflict".into()))?;
                let conflict = &pending.conflicts[index];
                if !matches!(choice, Resolution::Custom) && record.is_some() {
                    return Err(Error::Invalid(
                        "record is only accepted for custom resolution".into(),
                    ));
                }
                let selected = match choice {
                    Resolution::Ours => conflict.ours.clone(),
                    Resolution::Theirs => conflict.theirs.clone(),
                    Resolution::Base => conflict.base.clone(),
                    Resolution::Delete => None,
                    Resolution::Custom => Some(record.ok_or_else(|| {
                        Error::Invalid("custom resolution requires a record".into())
                    })?),
                };
                let selected = selected
                    .as_ref()
                    .map(|r| {
                        let mut target_binding = binding.clone();
                        let target = pending
                            .snapshot
                            .get(&dataset)
                            .ok_or_else(|| Error::Storage("missing merge dataset".into()))?;
                        target_binding.schema = schema::read(repo, &target.schema)?;
                        session.normalize(&target_binding, r)
                    })
                    .transpose()?;
                if selected.as_ref().is_some_and(|r| r.key != key) {
                    return Err(Error::Invalid("resolution cannot change key".into()));
                }
                let target = pending
                    .snapshot
                    .get_mut(&dataset)
                    .ok_or_else(|| Error::Storage("missing merge dataset".into()))?;
                merge::update(repo, target, &key, selected.as_ref())?;
                pending.conflicts.remove(index);
                let remaining = pending.conflicts.len();
                repo.save_state(&state)?;
                repo.commit()?;
                Ok(
                    json!({"resolved":true,"dataset":dataset,"key":key,"remaining_conflicts":remaining,"record":selected}),
                )
            }
            Command::MergeContinue => {
                ensure_clean(&changes)?;
                let pending = state
                    .merging
                    .clone()
                    .ok_or_else(|| Error::Conflict("no merge in progress".into()))?;
                if !pending.conflicts.is_empty() {
                    return Err(Error::Conflict(
                        "resolve all conflicts before continuing".into(),
                    ));
                }
                if pending.ours != before_head {
                    return Err(Error::Recovery("HEAD moved during merge".into()));
                }
                complete_or_stage(repo, &mut state, &baseline, pending, session.as_mut())
            }
            _ => Err(Error::Invalid(
                "operation does not use a working copy".into(),
            )),
        }
    }
}
fn identity(author: &str, message: &str) -> Result<()> {
    if author.trim().is_empty()
        || author.len() > 1024
        || message.trim().is_empty()
        || message.len() > 16_384
    {
        return Err(Error::Invalid(
            "author/message must be non-empty and within size limits".into(),
        ));
    }
    Ok(())
}
fn create_commit(
    repo: &dyn ObjectStore,
    snapshot: &Snapshot,
    parents: Vec<ObjectId>,
    author: &str,
    message: &str,
) -> Result<ObjectId> {
    identity(author, message)?;
    let commit = Commit {
        version: 1,
        parents,
        root: save(repo, "snapshot/v1", snapshot)?,
        author: author.into(),
        message: message.into(),
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
    };
    save(repo, "commit/v1", &commit)
}
fn snapshot_at(repo: &dyn ObjectStore, id: &ObjectId) -> Result<Snapshot> {
    load(repo, "snapshot/v1", &graph::commit(repo, id)?.root)
}
fn ensure_not_merging(state: &RepositoryState) -> Result<()> {
    if state.merging.is_some() {
        Err(Error::Conflict(
            "merge/revert in progress; resolve, continue or abort first".into(),
        ))
    } else {
        Ok(())
    }
}
fn ensure_clean(changes: &WorkingChanges) -> Result<()> {
    if changes.is_empty() {
        Ok(())
    } else {
        Err(Error::Dirty)
    }
}
fn record_size(record: &Record) -> usize {
    record.payload_bytes()
}
fn capture_dataset(
    repo: &Repository,
    binding: &Binding,
    session: &mut dyn WorkingCopyTransaction,
) -> Result<Dataset> {
    repo.bulk_write(|store| {
        let mut tree = tree::BulkBuilder::default();
        let mut records = 0;
        session.scan(binding, &mut |record| {
            let id = save(store, "record/v1", &record)?;
            tree.push(store, record.key, id)?;
            records += 1;
            Ok(())
        })?;
        Ok(Dataset {
            schema: schema::store(store, &binding.schema)?,
            root: tree.finish(store)?,
            records,
        })
    })
}
fn schema_changes(repo: &dyn ObjectStore, before: &Snapshot, after: &Snapshot) -> Result<Value> {
    let mut changes = Vec::new();
    for name in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        let a = before.get(name).map(|d| &d.schema);
        let b = after.get(name).map(|d| &d.schema);
        if a != b {
            changes.push(json!({"dataset":name,"before":a.map(|id|schema::read(repo,id)).transpose()?,"after":b.map(|id|schema::read(repo,id)).transpose()?}));
        }
    }
    Ok(json!(changes))
}
fn restore_schema_edits(
    repo: &dyn ObjectStore,
    state: &mut RepositoryState,
    baseline: &Snapshot,
    dirty: &BTreeMap<String, Schema>,
    session: &mut dyn WorkingCopyTransaction,
) -> Result<()> {
    if dirty.is_empty() {
        return Ok(());
    }
    for (name, schema) in dirty {
        let binding = state
            .bindings
            .get_mut(name)
            .ok_or_else(|| Error::Storage("missing binding".into()))?;
        binding.schema = schema.clone();
        binding.column_ids = session.column_ids(binding)?;
    }
    apply_snapshot(repo, state, baseline, baseline, session)
}
fn apply_snapshot(
    repo: &dyn ObjectStore,
    state: &mut RepositoryState,
    before: &Snapshot,
    after: &Snapshot,
    session: &mut dyn WorkingCopyTransaction,
) -> Result<()> {
    let mut rewritten = BTreeSet::new();
    for (name, dataset) in after {
        let binding = state
            .bindings
            .get_mut(name)
            .ok_or_else(|| Error::Unsupported(format!("no working-copy binding for {name}")))?;
        let schema: Schema = schema::read(repo, &dataset.schema)?;
        if schema != binding.schema {
            session.replace_schema(binding, &schema)?;
            binding.schema = schema;
            binding.column_ids = session.column_ids(binding)?;
            let mut batch = Vec::<Record>::new();
            let mut bytes = 0usize;
            tree::visit(repo, dataset.root.as_ref(), &mut |_, id| {
                let record: Record = load(repo, "record/v1", id)?;
                bytes += record_size(&record);
                batch.push(record);
                if batch.len() >= 1000 || bytes >= 8 * 1024 * 1024 {
                    session.write_many(
                        binding,
                        &batch
                            .iter()
                            .map(|r| (r.key.as_str(), Some(r)))
                            .collect::<Vec<_>>(),
                    )?;
                    batch.clear();
                    bytes = 0;
                }
                Ok(())
            })?;
            if !batch.is_empty() {
                session.write_many(
                    binding,
                    &batch
                        .iter()
                        .map(|r| (r.key.as_str(), Some(r)))
                        .collect::<Vec<_>>(),
                )?;
            }
            rewritten.insert(name.clone());
        }
    }
    let mut before = before.clone();
    let mut after = after.clone();
    before.retain(|n, _| !rewritten.contains(n));
    after.retain(|n, _| !rewritten.contains(n));
    write_snapshot_diff(repo, state, &before, &after, session)
}
fn finish(
    repo: &Repository,
    session: &mut dyn WorkingCopyTransaction,
    before_head: &ObjectId,
    state: &RepositoryState,
) -> Result<()> {
    let operation = uuid::Uuid::new_v4().to_string();
    session.clear_dirty()?;
    session.mark(&operation, state.head()?)?;
    repo.prepare(&PendingOperation {
        id: operation,
        before_head: before_head.clone(),
        after: state.clone(),
    })?;
    // Objects and journal must be durable BEFORE the database becomes visible.
    repo.commit()?;
    if session.commit().is_err() {
        return Err(Error::Recovery(
            "PostGIS COMMIT outcome is uncertain; run recover before retrying".into(),
        ));
    }
    repo.begin()?;
    repo.save_state(state)?;
    repo.clear_pending()?;
    repo.commit()?;
    Ok(())
}
fn complete_or_stage(
    repo: &Repository,
    state: &mut RepositoryState,
    baseline: &Snapshot,
    pending: MergeState,
    session: &mut dyn WorkingCopyTransaction,
) -> Result<Value> {
    identity(&pending.author, &pending.message)?;
    if !pending.conflicts.is_empty() {
        let count = pending.conflicts.len();
        state.merging = Some(pending);
        repo.save_state(state)?;
        repo.commit()?;
        return Ok(
            json!({"state":"merging","conflicts":count,"working_copy_changed":false,"head":state.head()?}),
        );
    }
    validate_merge_records(repo, state, baseline, &pending.snapshot, session)?;
    let before = state.head()?.clone();
    let id = create_commit(
        repo,
        &pending.snapshot,
        pending.parents,
        &pending.author,
        &pending.message,
    )?;
    apply_snapshot(repo, state, baseline, &pending.snapshot, session)?;
    state.branches.insert(state.branch.clone(), id.clone());
    state.merging = None;
    finish(repo, session, &before, state)?;
    Ok(json!({"commit":id,"state":"normal","fast_forward":false}))
}
fn page<T: serde::Serialize>(key: &str, values: &[T], limit: usize) -> Value {
    let limit = limit.clamp(1, 1000);
    json!({key:values.iter().take(limit).collect::<Vec<_>>(),"total":values.len(),"truncated":values.len()>limit})
}
fn status(state: &RepositoryState, changes: &WorkingChanges, limit: usize) -> Value {
    json!({"repository_id":state.repository_id,"branch":state.branch,"head":state.head().ok(),"clean":changes.is_empty(),
        "state":if state.merging.is_some(){"merging"}else{"normal"},
        "unresolved_conflicts":state.merging.as_ref().map(|m|m.conflicts.len()).unwrap_or(0),
        "datasets":state.bindings.keys().collect::<Vec<_>>(),
        "summary":{"inserted":changes.inserted,"updated":changes.len()-changes.inserted-changes.deleted,"deleted":changes.deleted},
        "diff":changes.page(limit)})
}
fn fsck(repo: &Repository, state: &RepositoryState) -> Result<Value> {
    let objects = repo.verify_objects()?;
    let mut visited = BTreeSet::new();
    for head in state.branches.values() {
        visited.extend(graph::ancestors(repo, head)?);
    }
    let mut checked_snapshots = BTreeSet::new();
    let mut checked_datasets = BTreeSet::new();
    for id in &visited {
        let commit = graph::commit(repo, id)?;
        if !checked_snapshots.insert(commit.root.clone()) {
            continue;
        }
        for dataset in snapshot_at(repo, id)?.values() {
            if !checked_datasets.insert((
                dataset.schema.clone(),
                dataset.root.clone(),
                dataset.records,
            )) {
                continue;
            }
            let schema: Schema = schema::read(repo, &dataset.schema)?;
            let mut count = 0;
            tree::visit(repo, dataset.root.as_ref(), &mut |key, id| {
                let row: Record = load(repo, "record/v1", id)?;
                if key != row.key {
                    return Err(Error::Storage("tree key/record key mismatch".into()));
                }
                schema.validate(&row)?;
                count += 1;
                Ok(())
            })?;
            if count != dataset.records {
                return Err(Error::Storage("dataset record count mismatch".into()));
            }
        }
    }
    Ok(
        json!({"ok":true,"objects":objects,"reachable_commits":visited.len(),"snapshots":checked_snapshots.len()}),
    )
}
