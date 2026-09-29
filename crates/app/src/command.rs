use geoledger_core::Record;
use serde::{Deserialize, Serialize};
fn head() -> String {
    "HEAD".into()
}
fn author() -> String {
    "unknown".into()
}
fn limit() -> usize {
    100
}
fn public() -> String {
    "public".into()
}

/// The same command contract is used by the library, CLI, HTTP and gRPC.
/// Destructive operations require an explicit flag on every transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Upgrade,
    AlterSchema {
        dataset: String,
        change: geoledger_core::schema::SchemaEdit,
        #[serde(default = "author")]
        author: String,
        #[serde(default)]
        message: Option<String>,
    },
    Schema {
        dataset: String,
        #[serde(default = "head")]
        reference: String,
    },
    Init {
        #[serde(default = "author")]
        author: String,
    },
    Import {
        dataset: String,
        #[serde(default = "public")]
        schema: String,
        table: String,
        #[serde(default = "author")]
        author: String,
        #[serde(default)]
        message: Option<String>,
    },
    Status {
        #[serde(default = "limit")]
        limit: usize,
    },
    Diff {
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        to: Option<String>,
        #[serde(default = "limit")]
        limit: usize,
    },
    Commit {
        message: String,
        #[serde(default = "author")]
        author: String,
    },
    Log {
        #[serde(default = "head")]
        reference: String,
        #[serde(default = "limit")]
        limit: usize,
    },
    Show {
        #[serde(default = "head")]
        reference: String,
        #[serde(default)]
        dataset: Option<String>,
        #[serde(default)]
        key: Option<String>,
    },
    Branches,
    Branch {
        name: String,
        #[serde(default = "head")]
        from: String,
    },
    Switch {
        branch: String,
    },
    Reset {
        target: String,
        #[serde(default)]
        hard: bool,
    },
    Restore {
        #[serde(default)]
        discard: bool,
    },
    Merge {
        source: String,
        #[serde(default = "author")]
        author: String,
        #[serde(default)]
        message: Option<String>,
    },
    Revert {
        target: String,
        #[serde(default = "author")]
        author: String,
        #[serde(default)]
        message: Option<String>,
    },
    Conflicts {
        #[serde(default = "limit")]
        limit: usize,
    },
    Resolve {
        dataset: String,
        key: String,
        choice: Resolution,
        #[serde(default)]
        record: Option<Record>,
    },
    MergeContinue,
    MergeAbort,
    Recover,
    Fsck,
    Reflog {
        #[serde(default = "limit")]
        limit: usize,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Ours,
    Theirs,
    Base,
    Delete,
    Custom,
}
