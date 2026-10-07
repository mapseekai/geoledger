//! Embedded managed-table collaboration. All host callbacks share this transaction.
use crate::*;
mod draft;
mod table;
use draft::*;
use table::*;

const PAGE_BYTES: usize = 1024 * 1024;
const UPLOAD_BYTES: usize = 16 * 1024 * 1024;
const FEATURE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PAGE: usize = 1000;

#[derive(Clone, Debug)]
pub struct Scope {
    pub subject: String,
    pub tenant: String,
    pub project: String,
    pub dataset: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableBinding {
    pub schema: String,
    pub table: String,
    pub id_column: String,
    pub geometry_column: String,
    pub srid: i32,
}
/// Host authorization and metadata changes execute inside the publication transaction.
pub trait Host {
    fn authorize(
        &self,
        tx: &mut Transaction<'_>,
        scope: &Scope,
        write: bool,
    ) -> Result<TableBinding>;
    fn published(
        &self,
        tx: &mut Transaction<'_>,
        scope: &Scope,
        revision: i64,
        schema_changed: bool,
    ) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum CollaborationCommand {
    Register,
    Head,
    Snapshot {
        epoch: String,
        revision: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    Changes {
        epoch: String,
        from: String,
        to: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    OpenDraft {
        epoch: String,
        base_revision: String,
        request_id: String,
    },
    SaveDelta {
        epoch: String,
        workspace: String,
        expected_version: i64,
        request_id: String,
        operations: Vec<Value>,
    },
    Preview {
        epoch: String,
        workspace: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    Resolve {
        epoch: String,
        workspace: String,
        expected_version: i64,
        expected_head: String,
        request_id: Option<String>,
        resolutions: Vec<Resolution>,
    },
    Rebase {
        epoch: String,
        workspace: String,
        expected_version: i64,
        expected_head: String,
        request_id: Option<String>,
    },
    DraftChanges {
        epoch: String,
        workspace: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    Publish {
        epoch: String,
        workspace: String,
        expected_version: i64,
        request_id: String,
        #[serde(default)]
        message: String,
    },
    CommitResult {
        epoch: String,
        request_id: String,
    },
    History {
        epoch: String,
        after: Option<String>,
        limit: Option<usize>,
    },
    Commit {
        epoch: String,
        revision: String,
        after: Option<String>,
        limit: Option<usize>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub id: String,
    pub choice: String,
    #[serde(default)]
    pub feature: Value,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Column {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    nullable: bool,
    editable: bool,
}
struct Dataset {
    epoch: String,
    head: i64,
    binding: TableBinding,
}
impl CollaborationCommand {
    fn write(&self) -> bool {
        !matches!(
            self,
            Self::Head
                | Self::Snapshot { .. }
                | Self::Changes { .. }
                | Self::Preview { .. }
                | Self::DraftChanges { .. }
                | Self::CommitResult { .. }
                | Self::History { .. }
                | Self::Commit { .. }
        )
    }
}
impl CenterApplication {
    /// Execute a trusted, scoped command. Call this synchronous API on a bounded blocking worker.
    pub fn collaborate(
        &self,
        scope: &Scope,
        command: CollaborationCommand,
        host: &impl Host,
    ) -> Result<Value> {
        self.collaborate_inner(scope, command, host)
            .map_err(Error::invalid_table_value)
    }
    fn collaborate_inner(
        &self,
        scope: &Scope,
        command: CollaborationCommand,
        host: &impl Host,
    ) -> Result<Value> {
        for value in [
            &scope.subject,
            &scope.tenant,
            &scope.project,
            &scope.dataset,
        ] {
            text(value, 256)?;
        }
        let payload = serde_json::to_value(&command).map_err(Error::invalid_json)?;
        if payload.to_string().len() > UPLOAD_BYTES {
            return Err(Error::new(413, "upload chunk exceeds 16 MiB"));
        }
        let mut c = self.connect()?;
        let mut t = c.transaction()?;
        let format: i32 = t
            .query_one(
                "SELECT version FROM _geoledger_center.format WHERE singleton",
                &[],
            )?
            .get(0);
        if format != FORMAT_VERSION {
            return Err(Error::new(409, "unsupported center schema format"));
        }
        let binding = host.authorize(&mut t, scope, command.write())?;
        validate_binding(&binding)?;
        if matches!(command, CollaborationCommand::Register) {
            register(&mut t, scope, &binding)?;
        }
        // Every command observes a fixed head; writers serialize on this Dataset only.
        let suffix = if command.write() { " FOR UPDATE" } else { "" };
        let row = t.query_opt(&format!("SELECT epoch::text,head,source_schema,source_table,id_column,geometry_column,srid FROM _geoledger_center.collab_datasets WHERE tenant=$1 AND project=$2 AND dataset=$3{suffix}"), &[&scope.tenant, &scope.project, &scope.dataset])?.ok_or_else(|| {
            let mut e = Error::new(409, "dataset is not registered"); e.body["error"]["code"] = json!("not_registered"); e
        })?;
        let d = Dataset {
            epoch: row.get(0),
            head: row.get(1),
            binding: TableBinding {
                schema: row.get(2),
                table: row.get(3),
                id_column: row.get(4),
                geometry_column: row.get(5),
                srid: row.get(6),
            },
        };
        if d.binding != binding {
            return Err(Error::new(
                409,
                "registered dataset binding changed; re-register in a fresh epoch",
            ));
        }
        if let Some(epoch) = payload.get("epoch").and_then(Value::as_str) {
            id(epoch)?;
            if epoch != d.epoch {
                let mut e = Error::new(
                    409,
                    "dataset epoch changed; preserve local edits and rebuild the base",
                );
                e.body["error"]["code"] = json!("epoch_mismatch");
                return Err(e);
            }
        }
        let request = payload.get("request_id").and_then(Value::as_str);
        if let Some(request) = request {
            id(request)?;
        }
        if request.is_some() && !matches!(command, CollaborationCommand::CommitResult { .. })
            && let Some(row) = t.query_opt("SELECT payload::text,result::text FROM _geoledger_center.collab_requests WHERE epoch=$1::text::uuid AND subject=$2 AND request_id=$3::text::uuid", &[&d.epoch,&scope.subject,&request])? {
                let original: Value = parse(&row.get::<_,String>(0))?;
                if original != payload { return Err(Error::new(409, "request_id was already used with different content")); }
                let result = parse(&row.get::<_,String>(1))?; t.commit()?; return Ok(result);
        }
        let result = match command {
            CollaborationCommand::Head | CollaborationCommand::Register => head(&mut t, &d)?,
            CollaborationCommand::Snapshot {revision, after, limit, ..} => read_rows(&mut t,&d,0,revision_number(&revision,d.head)?,after,limit,"snapshot")?,
            CollaborationCommand::Changes {from,to,after,limit,..} => read_rows(&mut t,&d,revision_number(&from,d.head)?,revision_number(&to,d.head)?,after,limit,"changes")?,
            CollaborationCommand::Commit {revision,after,limit,..} => { let r=revision_number(&revision,d.head)?; if r==0 {return Err(bad());} read_rows(&mut t,&d,r-1,r,after,limit,"commit")? },
            CollaborationCommand::OpenDraft {base_revision,..} => {
                let base = revision_number(&base_revision,d.head)?; let workspace = Uuid::new_v4().to_string();
                let schema = schema_at(&mut t,&d,base)?;
                t.execute("INSERT INTO _geoledger_center.collab_workspaces(epoch,id,owner,base_revision,columns) VALUES($1::text::uuid,$2::text::uuid,$3,$4,$5::text::jsonb)",&[&d.epoch,&workspace,&scope.subject,&base,&json!(schema).to_string()])?;
                json!({"workspace":workspace,"version":0,"base_revision":base.to_string()})
            }
            CollaborationCommand::SaveDelta {workspace,expected_version,operations,..} => save_delta(&mut t,&d,scope,&workspace,expected_version,operations)?,
            CollaborationCommand::DraftChanges {workspace,after,limit,..} => draft_page(&mut t,&d,scope,&workspace,after,limit)?,
            CollaborationCommand::Preview {workspace,after,limit,..} => preview(&mut t,&d,scope,&workspace,after,limit)?,
            CollaborationCommand::Resolve {workspace,expected_version,expected_head,resolutions,..} => resolve_draft(&mut t,&d,scope,&workspace,expected_version,revision_number(&expected_head,d.head)?,resolutions)?,
            CollaborationCommand::Rebase {workspace,expected_version,expected_head,..} => rebase_draft(&mut t,&d,scope,&workspace,expected_version,revision_number(&expected_head,d.head)?)?,
            CollaborationCommand::Publish {workspace,expected_version,message,..} => publish_draft(&mut t,&d,scope,&workspace,expected_version,&message,host)?,
            CollaborationCommand::CommitResult {request_id,..} => {
                if let Some(row)=t.query_opt("SELECT result::text FROM _geoledger_center.collab_requests WHERE epoch=$1::text::uuid AND subject=$2 AND request_id=$3::text::uuid AND payload->>'op'='publish'",&[&d.epoch,&scope.subject,&request_id])? {parse(&row.get::<_,String>(0))?} else {json!({"status":"unknown"})}
            }
            CollaborationCommand::History {after,limit,..} => history(&mut t,&d,after,limit)?,
        };
        if result.to_string().len() > UPLOAD_BYTES {
            return Err(Error::new(
                413,
                "response exceeds 16 MiB; reduce the page or feature size",
            ));
        }
        if let Some(request) = request.filter(|_| payload["op"] != "commit_result") {
            t.execute("INSERT INTO _geoledger_center.collab_requests(epoch,subject,request_id,payload,result) VALUES($1::text::uuid,$2,$3::text::uuid,$4::text::jsonb,$5::text::jsonb)",&[&d.epoch,&scope.subject,&request,&payload.to_string(),&result.to_string()])?;
        }
        t.commit()?;
        Ok(result)
    }
}
fn parse<T: DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(Error::stored_json)
}
fn revision_number(s: &str, head: i64) -> Result<i64> {
    let n = s.parse::<i64>().map_err(|_| bad())?;
    if n < 0 || n > head || n.to_string() != s {
        return Err(Error::new(409, "revision unavailable"));
    }
    Ok(n)
}
fn page_limit(limit: Option<usize>) -> Result<usize> {
    let n = limit.unwrap_or(200);
    if n == 0 || n > MAX_PAGE {
        Err(bad())
    } else {
        Ok(n)
    }
}
// The opaque cursor includes every resource/version input. A cursor is a position,
// not an authorization credential; the host authorizes every replay.
fn cursor(context: &Value, key: &str) -> String {
    json!({"v":1,"context":context,"key":key}).to_string()
}
fn after_key(after: Option<String>, context: &Value) -> Result<String> {
    let Some(after) = after else {
        return Ok(String::new());
    };
    if after.len() > 4096 {
        return Err(bad());
    }
    let value: Value = serde_json::from_str(&after).map_err(Error::invalid_json)?;
    if value["v"] != 1 || value["context"] != *context {
        return Err(Error::new(
            409,
            "cursor does not match this dataset or version",
        ));
    }
    value["key"].as_str().map(str::to_owned).ok_or_else(bad)
}
fn schema_at(t: &mut Transaction<'_>, d: &Dataset, revision: i64) -> Result<Vec<Column>> {
    parse(&t.query_one("SELECT columns::text FROM _geoledger_center.collab_schemas WHERE epoch=$1::text::uuid AND revision<=$2 ORDER BY revision DESC LIMIT 1",&[&d.epoch,&revision])?.get::<_,String>(0))
}
fn head(t: &mut Transaction<'_>, d: &Dataset) -> Result<Value> {
    Ok(
        json!({"epoch":d.epoch,"revision":d.head.to_string(),"schema":schema_at(t,d,d.head)?,"id_column":d.binding.id_column,"geometry_column":d.binding.geometry_column,"srid":d.binding.srid}),
    )
}
fn read_rows(
    t: &mut Transaction<'_>,
    d: &Dataset,
    from: i64,
    to: i64,
    after: Option<String>,
    limit: Option<usize>,
    kind: &str,
) -> Result<Value> {
    if from > to {
        return Err(bad());
    }
    let context = json!([d.epoch, kind, from.to_string(), to.to_string()]);
    let after = after_key(after, &context)?;
    let limit = page_limit(limit)?;
    let filter = if kind == "snapshot" {
        "feature IS NOT NULL AND $4::bigint>=0"
    } else {
        "revision>$4"
    };
    let range = if kind == "snapshot" {
        ""
    } else {
        " AND revision>$4"
    };
    // Candidate IDs are keyset-paged; no OFFSET and no live-table reads.
    let rows=t.query(&format!("WITH candidates AS (SELECT id,feature FROM (SELECT DISTINCT ON(id) id,revision,feature FROM _geoledger_center.collab_rows WHERE epoch=$1::text::uuid AND revision<=$2 AND id>$3{range} ORDER BY id,revision DESC) r WHERE {filter} ORDER BY id LIMIT $5), sized AS (SELECT id,feature,coalesce(sum(coalesce(octet_length(feature::text),0)+octet_length(id)+64) OVER(ORDER BY id ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING),0) bytes FROM candidates) SELECT id,feature::text,(SELECT count(*) FROM candidates) FROM sized WHERE bytes<1048576 ORDER BY id"),&[&d.epoch,&to,&after,&from,&((limit+1) as i64)])?;
    let columns = schema_at(t, d, to)?;
    let mut items = Vec::new();
    let mut bytes = 0;
    let mut last = after;
    let mut done = rows
        .first()
        .is_none_or(|r| r.get::<_, i64>(2) as usize == rows.len());
    for row in rows {
        let key: String = row.get(0);
        let f: Option<String> = row.get(1);
        let mut feature = f
            .map(|s| parse::<Value>(&s))
            .transpose()?
            .unwrap_or(Value::Null);
        if let Some(properties) = feature.get_mut("properties").and_then(Value::as_object_mut) {
            properties.retain(|key, _| columns.iter().any(|c| &c.name == key));
            for c in &columns {
                properties.entry(c.name.clone()).or_insert(Value::Null);
            }
        }
        let value = if kind == "snapshot" {
            feature
        } else {
            json!({"id":key,"feature":feature})
        };
        let size = value.to_string().len();
        if items.len() == limit || (!items.is_empty() && bytes + size > PAGE_BYTES) {
            done = false;
            break;
        }
        bytes += size;
        items.push(value);
        last = key;
    }
    let mut value = json!({"epoch":d.epoch,"revision":to.to_string(),"from":from.to_string(),"to":to.to_string(),"schema":columns,"next_after":if done {None}else{Some(cursor(&context,&last))},"done":done});
    value[if kind == "snapshot" {
        "features"
    } else {
        "changes"
    }] = json!(items);
    Ok(value)
}
fn history(
    t: &mut Transaction<'_>,
    d: &Dataset,
    after: Option<String>,
    limit: Option<usize>,
) -> Result<Value> {
    let context = json!([d.epoch, "history"]);
    let key = after_key(after, &context)?;
    let before = if key.is_empty() {
        i64::MAX
    } else {
        revision_number(&key, d.head)?
    };
    let limit = page_limit(limit)?;
    let rows=t.query("SELECT revision,subject,message,created_at::text,changes FROM _geoledger_center.collab_commits WHERE epoch=$1::text::uuid AND revision<$2 ORDER BY revision DESC LIMIT $3",&[&d.epoch,&before,&((limit+1) as i64)])?;
    let next = if rows.len() > limit {
        Some(cursor(
            &context,
            &rows[limit - 1].get::<_, i64>(0).to_string(),
        ))
    } else {
        None
    };
    Ok(
        json!({"commits":rows.into_iter().take(limit).map(|r|json!({"revision":r.get::<_,i64>(0).to_string(),"subject":r.get::<_,String>(1),"message":r.get::<_,String>(2),"created_at":r.get::<_,String>(3),"changes":r.get::<_,i64>(4)})).collect::<Vec<_>>(),"next_after":next}),
    )
}
