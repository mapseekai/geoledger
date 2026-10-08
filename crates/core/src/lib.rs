//! Storage- and transport-independent three-way feature merge.
#![forbid(unsafe_code)]
pub mod merge;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub key: String,
    pub fields: BTreeMap<String, Cell>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Cell {
    Null,
    Text(String),
    Geometry(String),
}
