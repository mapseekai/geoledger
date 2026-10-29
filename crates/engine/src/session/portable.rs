//! Logical export, import and verification of a whole database as JSON lines.
//!
//! File layout (one JSON object per line):
//! 1. header `{"geoledger_export":1,"format":19,"backend":"sqlite"}`
//! 2. per table, in foreign-key order: `{"table":..,"columns":[..]}`, one
//!    `{"r":[..]}` line per row, then `{"table_end":..,"rows":N,"digest":hex}`
//! 3. trailer `{"end":true,"checksum":hex}` where checksum is the SHA-256 hash of
//!    every preceding byte.
//!
//! Table digests are order-independent (sum of per-row SHA-256 hashes modulo
//! 2^256), so SQLite and PostgreSQL exports of the same data compare equal even
//! though each backend pages rows in its own collation order. They detect
//! corruption and incomplete copies; they are not a signature.
use super::{Backend, Cell, Client, Parameter, Row, SqlTransaction};
use crate::{Error, FORMAT_VERSION, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::io::{BufRead, Write};

/// Current export file version.
pub const EXPORT_VERSION: i64 = 1;
const PAGE: i64 = 1000;
const INSERT_BATCH: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Text,
    Int,
    Bool,
}
struct Table {
    name: &'static str,
    /// (column, kind, nullable); the first `key` columns are the primary key.
    columns: &'static [(&'static str, Kind, bool)],
    key: usize,
}
use Kind::{Bool, Int, Text};
/// Portable version tables of format 19 in foreign-key order. Spatial indexes
/// and gl_format are rebuilt; business-table binding state stays with its source.
const TABLES: &[Table] = &[
    Table {
        name: "gl_projects",
        columns: &[
            ("id", Text, false),
            ("name", Text, false),
            ("head", Int, false),
            ("state", Text, false),
        ],
        key: 1,
    },
    Table {
        name: "gl_project_members",
        columns: &[
            ("project", Text, false),
            ("subject", Text, false),
            ("role", Text, false),
            ("removed", Bool, false),
        ],
        key: 2,
    },
    Table {
        name: "gl_datasets",
        columns: &[
            ("project", Text, false),
            ("id", Text, false),
            ("name", Text, false),
            ("geometry_type", Text, false),
            ("coordinate_dimension", Int, false),
            ("crs", Text, false),
            ("source_revision", Int, false),
        ],
        key: 2,
    },
    Table {
        name: "gl_workspaces",
        columns: &[
            ("project", Text, false),
            ("id", Text, false),
            ("owner", Text, false),
            ("base_revision", Int, false),
            ("version", Int, false),
            ("status", Text, false),
        ],
        key: 2,
    },
    Table {
        name: "gl_commits",
        columns: &[
            ("project", Text, false),
            ("revision", Int, false),
            ("workspace", Text, false),
            ("subject", Text, false),
            ("message", Text, false),
            ("created_at", Text, false),
        ],
        key: 2,
    },
    Table {
        name: "gl_schema_history",
        columns: &[
            ("project", Text, false),
            ("dataset", Text, false),
            ("revision", Int, false),
            ("definition", Text, false),
        ],
        key: 3,
    },
    Table {
        name: "gl_workspace_schemas",
        columns: &[
            ("project", Text, false),
            ("workspace", Text, false),
            ("dataset", Text, false),
            ("definition", Text, false),
        ],
        key: 3,
    },
    Table {
        name: "gl_history",
        columns: &[
            ("project", Text, false),
            ("dataset", Text, false),
            ("feature_id", Text, false),
            ("valid_from", Int, false),
            ("valid_to", Int, true),
            ("properties", Text, true),
            ("geometry_json", Text, true),
        ],
        key: 4,
    },
    Table {
        name: "gl_commit_changes",
        columns: &[
            ("project", Text, false),
            ("revision", Int, false),
            ("dataset", Text, false),
            ("feature_id", Text, false),
            ("had_before", Bool, false),
            ("has_after", Bool, false),
        ],
        key: 4,
    },
    Table {
        name: "gl_workspace_changes",
        columns: &[
            ("project", Text, false),
            ("workspace", Text, false),
            ("dataset", Text, false),
            ("feature_id", Text, false),
            ("resolved_head", Int, true),
            ("resolution_stale", Bool, false),
            ("properties", Text, true),
            ("geometry_json", Text, true),
        ],
        key: 4,
    },
    Table {
        name: "gl_idempotency",
        columns: &[
            ("project", Text, false),
            ("subject", Text, false),
            ("request_id", Text, false),
            ("payload", Text, false),
            ("result", Text, false),
        ],
        key: 3,
    },
    Table {
        name: "gl_audit_events",
        columns: &[
            ("id", Int, false),
            ("project", Text, false),
            ("subject", Text, false),
            ("action", Text, false),
            ("detail", Text, false),
            ("created_at", Text, false),
        ],
        key: 1,
    },
    Table {
        name: "gl_current_features",
        columns: &[
            ("project", Text, false),
            ("dataset", Text, false),
            ("feature_id", Text, false),
            ("valid_from", Int, false),
            ("properties", Text, false),
            ("geometry_json", Text, true),
        ],
        key: 3,
    },
];

/// Row count and order-independent digest of one table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSummary {
    pub table: String,
    pub rows: u64,
    pub digest: String,
}
/// Content summary of a database or export file at one storage format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSummary {
    pub format: i32,
    pub tables: Vec<TableSummary>,
}
impl DataSummary {
    /// Rows of one table, 0 when absent.
    pub fn rows(&self, table: &str) -> u64 {
        self.tables
            .iter()
            .find(|t| t.table == table)
            .map_or(0, |t| t.rows)
    }
}

/// Order-independent multiset digest.
struct Digest {
    sum: [u8; 32],
    rows: u64,
}
impl Digest {
    fn new() -> Self {
        Self {
            sum: [0; 32],
            rows: 0,
        }
    }
    fn add(&mut self, table: &str, row: &str) {
        let mut h = Sha256::new();
        h.update(table.as_bytes());
        h.update([0]);
        h.update(row.as_bytes());
        let mut carry = 0u16;
        for (s, b) in self.sum.iter_mut().zip(h.finalize()) {
            let v = u16::from(*s) + u16::from(b) + carry;
            *s = (v & 0xff) as u8;
            carry = v >> 8;
        }
        self.rows += 1;
    }
    fn summary(&self, table: &str) -> TableSummary {
        TableSummary {
            table: table.to_owned(),
            rows: self.rows,
            digest: hex(&self.sum),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
impl Parameter for Cell {
    fn value(&self) -> Cell {
        self.clone()
    }
}
fn corrupt(detail: &str) -> Error {
    Error::new(400, &format!("invalid export file: {detail}"))
}
fn io_error(e: std::io::Error) -> Error {
    Error::new(500, "export stream failed").caused_by(e)
}
/// Canonical JSON for a stored cell; booleans are normalized across backends.
fn encode(table: &Table, column: usize, cell: &Cell) -> Result<Value> {
    let (name, kind, nullable) = table.columns[column];
    let bad = || {
        Error::new(
            500,
            &format!("unexpected stored value in {}.{name}", table.name),
        )
    };
    Ok(match (kind, cell) {
        (_, Cell::Null) if nullable => Value::Null,
        (Text, Cell::Text(s)) => Value::String(s.clone()),
        (Int, Cell::Integer(v)) => json!(v),
        (Bool, Cell::Bool(v)) => Value::Bool(*v),
        (Bool, Cell::Integer(v @ (0 | 1))) => Value::Bool(*v == 1),
        _ => return Err(bad()),
    })
}
fn decode(table: &Table, column: usize, value: &Value) -> Result<Cell> {
    let (name, kind, nullable) = table.columns[column];
    Ok(match (kind, value) {
        (_, Value::Null) if nullable => Cell::Null,
        (Text, Value::String(s)) => Cell::Text(s.clone()),
        (Int, Value::Number(n)) => Cell::Integer(
            n.as_i64()
                .ok_or_else(|| corrupt(&format!("{}.{name} is not an integer", table.name)))?,
        ),
        (Bool, Value::Bool(v)) => Cell::Bool(*v),
        _ => {
            return Err(corrupt(&format!(
                "{}.{name} has an unexpected value",
                table.name
            )));
        }
    })
}

fn emit(out: &mut dyn Write, checksum: &mut Sha256, value: &Value) -> Result<()> {
    let mut text = serde_json::to_string(value)
        .map_err(|e| Error::new(500, "export encoding failed").caused_by(e))?;
    text.push('\n');
    checksum.update(text.as_bytes());
    out.write_all(text.as_bytes()).map_err(io_error)
}

fn current_page(t: &mut SqlTransaction, last: Option<&[Cell]>) -> Result<Vec<Row>> {
    let cursor = last
        .map(|cells| -> Result<(String, String, String)> {
            let text = |i| match &cells[i] {
                Cell::Text(v) => Ok(v.clone()),
                _ => Err(corrupt("current feature cursor")),
            };
            Ok((text(0)?, text(1)?, text(2)?))
        })
        .transpose()?
        .unwrap_or_default();
    let mut pair = (cursor.0, cursor.1);
    let mut strict = false;
    loop {
        let comparison = if strict { ">" } else { ">=" };
        let Some(dataset) = t.query_opt(&format!("SELECT project,id,storage_table,postgis_source,source_revision FROM gl_datasets WHERE (project,id){comparison}($1,$2) ORDER BY project,id LIMIT 1"), &[&pair.0,&pair.1])? else { return Ok(Vec::new()); };
        let project: String = dataset.get(0usize)?;
        let id: String = dataset.get(1usize)?;
        let after = if project == pair.0 && id == pair.1 {
            cursor.2.as_str()
        } else {
            ""
        };
        let rows = if let Some(table) = dataset.get::<_, Option<String>>(2usize)? {
            super::dataset::check_table_name(&table)?;
            t.query(&format!("SELECT CAST($1 AS text),CAST($2 AS text),feature_id,valid_from,properties,geometry_json FROM {table} WHERE feature_id>$3 ORDER BY feature_id LIMIT {PAGE}"), &[&project,&id,&after])?
        } else if let Some(source) = dataset.get::<_, Option<String>>(3usize)? {
            let source: super::tables::Source = crate::codec::stored(&source)?;
            t.ensure_source_clean(&project, &id, &source)?;
            let table = super::tables::relation(&source.table);
            let key = super::tables::quote(&source.table.id_column);
            let geom = super::tables::quote(&source.table.geometry_column);
            let start: i64 = dataset.get(4usize)?;
            let properties = super::tables::source_properties("$5", "$6");
            t.query(&format!("SELECT CAST($1 AS text),CAST($2 AS text),t.{key}::text COLLATE \"C\",coalesce(v.valid_from,$4::bigint),{properties}::text,ST_AsGeoJSON(t.{geom},17,0) FROM {table} t LEFT JOIN gl_source_versions v ON v.project=$1 AND v.dataset=$2 AND v.feature_id=t.{key}::text COLLATE \"C\" WHERE t.{key}::text COLLATE \"C\">$3 ORDER BY t.{key}::text COLLATE \"C\" LIMIT {PAGE}"), &[&project,&id,&after,&start,&source.table.id_column,&source.table.geometry_column])?
        } else {
            Vec::new()
        };
        if !rows.is_empty() {
            return Ok(rows);
        }
        pair = (project, id);
        strict = true;
    }
}
/// Stream every table inside one consistent read transaction. Returns the
/// summary that the trailer and table_end lines carry.
pub(crate) fn export(
    t: &mut SqlTransaction,
    backend: &str,
    out: &mut dyn Write,
) -> Result<DataSummary> {
    let format = t
        .query_one("SELECT version FROM gl_format WHERE singleton=true", &[])?
        .get::<_, i32>(0usize)?;
    if format != FORMAT_VERSION {
        return Err(Error::new(409, "unsupported storage format"));
    }
    let mut checksum = Sha256::new();
    emit(
        out,
        &mut checksum,
        &json!({"geoledger_export":EXPORT_VERSION,"format":format,"backend":backend}),
    )?;
    let mut tables = Vec::new();
    for table in TABLES {
        let names: Vec<&str> = table.columns.iter().map(|c| c.0).collect();
        emit(
            out,
            &mut checksum,
            &json!({"table":table.name,"columns":names}),
        )?;
        let columns = names.join(",");
        let key = names[..table.key].join(",");
        let placeholders = (1..=table.key)
            .map(|i| format!("${i}"))
            .collect::<Vec<_>>()
            .join(",");
        let first = format!(
            "SELECT {columns} FROM {} ORDER BY {key} LIMIT {PAGE}",
            table.name
        );
        let next = format!(
            "SELECT {columns} FROM {} WHERE ({key}) > ({placeholders}) ORDER BY {key} LIMIT {PAGE}",
            table.name
        );
        let mut digest = Digest::new();
        let mut last: Option<Vec<Cell>> = None;
        loop {
            let rows = if table.name == "gl_current_features" {
                current_page(t, last.as_deref())?
            } else {
                match &last {
                    None => t.query(&first, &[])?,
                    Some(cells) => {
                        let params: Vec<&dyn Parameter> =
                            cells.iter().map(|c| c as &dyn Parameter).collect();
                        t.query(&next, &params)?
                    }
                }
            };
            let count = rows.len();
            for row in rows {
                let cells = row.0;
                if cells.len() != table.columns.len() {
                    return Err(Error::new(500, "unexpected storage result"));
                }
                let values = cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| encode(table, i, c))
                    .collect::<Result<Vec<_>>>()?;
                let encoded = serde_json::to_string(&values)
                    .map_err(|e| Error::new(500, "export encoding failed").caused_by(e))?;
                digest.add(table.name, &encoded);
                let mut text = format!("{{\"r\":{encoded}}}");
                text.push('\n');
                checksum.update(text.as_bytes());
                out.write_all(text.as_bytes()).map_err(io_error)?;
                last = Some(cells.into_iter().take(table.key).collect());
            }
            // A current-feature page ends at each dataset boundary, even when short.
            if count == 0 || (count < PAGE as usize && table.name != "gl_current_features") {
                break;
            }
        }
        let summary = digest.summary(table.name);
        emit(
            out,
            &mut checksum,
            &json!({"table_end":table.name,"rows":summary.rows,"digest":summary.digest}),
        )?;
        tables.push(summary);
    }
    let sum = hex(&checksum.finalize());
    emit(out, &mut Sha256::new(), &json!({"end":true,"checksum":sum}))?;
    out.flush().map_err(io_error)?;
    Ok(DataSummary { format, tables })
}

/// Parse and fully validate an export stream, handing each batch of decoded
/// rows to `sink`. Any structural, digest or checksum error aborts.
pub(crate) fn read(
    input: &mut dyn BufRead,
    mut sink: impl FnMut(&'static str, &[(&'static str, Kind, bool)], Vec<Vec<Cell>>) -> Result<()>,
) -> Result<DataSummary> {
    let mut checksum = Sha256::new();
    let mut buffer = String::new();
    let mut next = |checksum: &mut Sha256, hash: bool| -> Result<Value> {
        buffer.clear();
        let n = input.read_line(&mut buffer).map_err(io_error)?;
        if n == 0 || !buffer.ends_with('\n') {
            return Err(corrupt("truncated"));
        }
        if hash {
            checksum.update(buffer.as_bytes());
        }
        serde_json::from_str(&buffer).map_err(|_| corrupt("malformed line"))
    };
    let header = next(&mut checksum, true)?;
    if header["geoledger_export"] != json!(EXPORT_VERSION) {
        return Err(corrupt("not a GeoLedger export (version 1)"));
    }
    let format = header["format"]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| corrupt("missing format"))?;
    if format != FORMAT_VERSION {
        return Err(Error::new(
            409,
            &format!(
                "export has storage format {format}; this server reads format {FORMAT_VERSION}"
            ),
        ));
    }
    let mut tables = Vec::new();
    for table in TABLES {
        let names: Vec<&str> = table.columns.iter().map(|c| c.0).collect();
        if next(&mut checksum, true)? != json!({"table":table.name,"columns":names}) {
            return Err(corrupt(&format!("expected table {}", table.name)));
        }
        let mut digest = Digest::new();
        let mut batch = Vec::with_capacity(INSERT_BATCH);
        let end = loop {
            let value = next(&mut checksum, true)?;
            let Some(row) = value.get("r").and_then(Value::as_array) else {
                break value;
            };
            if row.len() != table.columns.len() {
                return Err(corrupt(&format!("wrong column count in {}", table.name)));
            }
            let canonical = serde_json::to_string(row)
                .map_err(|e| Error::new(500, "export encoding failed").caused_by(e))?;
            digest.add(table.name, &canonical);
            batch.push(
                row.iter()
                    .enumerate()
                    .map(|(i, v)| decode(table, i, v))
                    .collect::<Result<Vec<_>>>()?,
            );
            if batch.len() == INSERT_BATCH {
                sink(table.name, table.columns, std::mem::take(&mut batch))?;
            }
        };
        if !batch.is_empty() {
            sink(table.name, table.columns, batch)?;
        }
        let summary = digest.summary(table.name);
        if end != json!({"table_end":table.name,"rows":summary.rows,"digest":summary.digest}) {
            return Err(corrupt(&format!(
                "row count or digest mismatch in {}",
                table.name
            )));
        }
        tables.push(summary);
    }
    let expected = hex(&checksum.clone().finalize());
    let trailer = next(&mut checksum, false)?;
    if trailer != json!({"end":true,"checksum":expected}) {
        return Err(corrupt("checksum mismatch"));
    }
    buffer.clear();
    if input.read_line(&mut buffer).map_err(io_error)? != 0 {
        return Err(corrupt("data after trailer"));
    }
    Ok(DataSummary { format, tables })
}

fn import_current_batch(t: &mut SqlTransaction, rows: Vec<Vec<Cell>>) -> Result<()> {
    let mut index = 0;
    while index < rows.len() {
        let (project, dataset) = match (&rows[index][0], &rows[index][1]) {
            (Cell::Text(project), Cell::Text(dataset)) => (project.as_str(), dataset.as_str()),
            _ => return Err(corrupt("current feature identity")),
        };
        let mut end = index + 1;
        while end < rows.len()
            && matches!((&rows[end][0],&rows[end][1]),(Cell::Text(p),Cell::Text(d)) if p==project && d==dataset)
        {
            end += 1;
        }
        if t.dataset_table(project, dataset)?.is_none() {
            t.create_dataset_table(project, dataset)?;
        }
        let table = t
            .dataset_table(project, dataset)?
            .ok_or_else(|| corrupt("missing current dataset"))?;
        let columns = t.geometry_columns();
        let values = (index..end)
            .map(|r| {
                let n = (r - index) * 4 + 1;
                format!(
                    "(${n},${},${},${},{})",
                    n + 1,
                    n + 2,
                    n + 3,
                    t.geometry_values(&format!("${}", n + 3))
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let params: Vec<&dyn Parameter> = rows[index..end]
            .iter()
            .flat_map(|r| r[2..6].iter().map(|c| c as &dyn Parameter))
            .collect();
        if matches!(t.0.backend, Backend::Postgis(_)) {
            t.query_one("SELECT set_config('geoledger.publication_table',($1::text::regclass::oid)::text,true)", &[&table])?;
        }
        t.with_dataset_writes(|t| t.execute(&format!("INSERT INTO {table}(feature_id,valid_from,properties,geometry_json,{columns}) VALUES {values}"),&params))?;
        if matches!(t.0.backend, Backend::Postgis(_)) {
            t.batch_execute("SET LOCAL geoledger.publication_table = ''")?;
        }
        index = end;
    }
    Ok(())
}

/// Import into an initialized, empty database inside one write transaction;
/// nothing is committed unless the whole file validates.
pub(crate) fn import(client: Client, input: &mut dyn BufRead) -> Result<DataSummary> {
    let postgis = matches!(client.backend, Backend::Postgis(_));
    let mut t = client.transaction(false)?;
    for table in TABLES {
        if table.name == "gl_current_features" {
            continue;
        }
        if t.query_opt(&format!("SELECT 1 FROM {} LIMIT 1", table.name), &[])?
            .is_some()
        {
            return Err(Error::new(
                409,
                "import requires an empty database; initialize a new one",
            ));
        }
    }
    let summary = read(input, |name, columns, rows| {
        if name == "gl_current_features" {
            return import_current_batch(&mut t, rows);
        }
        let width = columns.len();
        let mut names = columns.iter().map(|c| c.0).collect::<Vec<_>>().join(",");
        let geometry = columns.iter().position(|c| c.0 == "geometry_json");
        if geometry.is_some() {
            names.push(',');
            names.push_str(t.geometry_columns());
        }
        let values = (0..rows.len())
            .map(|r| {
                let mut cells = (1..=width)
                    .map(|c| format!("${}", r * width + c))
                    .collect::<Vec<_>>()
                    .join(",");
                if let Some(index) = geometry {
                    cells.push(',');
                    cells.push_str(&t.geometry_values(&format!("${}", r * width + index + 1)));
                }
                format!("({cells})")
            })
            .collect::<Vec<_>>()
            .join(",");
        let overriding = if postgis && name == "gl_audit_events" {
            " OVERRIDING SYSTEM VALUE"
        } else {
            ""
        };
        let sql = format!("INSERT INTO {name}({names}){overriding} VALUES {values}");
        let params: Vec<&dyn Parameter> =
            rows.iter().flatten().map(|c| c as &dyn Parameter).collect();
        t.execute(&sql, &params)
    })?;
    let mut after = (String::new(), String::new());
    loop {
        let datasets = t.query("SELECT project,id FROM gl_datasets WHERE (project,id)>($1,$2) ORDER BY project,id LIMIT 32", &[&after.0, &after.1])?;
        if datasets.is_empty() {
            break;
        }
        for row in datasets {
            let project: String = row.get(0usize)?;
            let dataset: String = row.get(1usize)?;
            if t.dataset_table(&project, &dataset)?.is_none() {
                t.create_dataset_table(&project, &dataset)?;
            }
            after = (project, dataset);
        }
    }
    if postgis {
        t.query_one(
            "SELECT setval(pg_get_serial_sequence('gl_audit_events','id'), (SELECT coalesce(max(id),0) FROM gl_audit_events)+1, false)",
            &[],
        )?;
    }
    t.commit()?;
    Ok(summary)
}
