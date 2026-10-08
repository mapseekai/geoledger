use crate::{Error, Result, bad, text};
use axum::http::HeaderMap;
use serde::Deserialize;
use std::collections::BTreeSet;
use subtle::ConstantTimeEq;
/// Operator-provided secrets are neither serializable nor Debug-printable.
pub struct Tokens(Vec<Token>);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    token: String,
    subject: String,
}
impl Tokens {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(bad());
        }
        let entries: Vec<Token> =
            serde_json::from_slice(bytes).map_err(|_| Error::new(400, "invalid token file"))?;
        if entries.is_empty() || entries.len() > 1024 {
            return Err(bad());
        }
        let mut tokens = BTreeSet::new();
        let mut subjects = BTreeSet::new();
        for e in &entries {
            text(&e.subject, 128)?;
            if e.token.len() < 43
                || e.token.len() > 256
                || !e.token.bytes().all(|b| b.is_ascii_graphic())
                || !tokens.insert(&e.token)
                || !subjects.insert(&e.subject)
            {
                return Err(Error::new(
                    400,
                    "tokens must be unique, 43-256 ASCII characters, with unique subjects",
                ));
            }
        }
        Ok(Self(entries))
    }
    pub(crate) fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        if headers.get_all("authorization").iter().count() != 1 {
            return None;
        }
        let token = headers
            .get("authorization")?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?;
        let mut subject = None;
        for e in &self.0 {
            if bool::from(e.token.as_bytes().ct_eq(token.as_bytes())) {
                subject = Some(e.subject.clone());
            }
        }
        subject
    }
}
