//! Static bearer credentials stored as SHA-256 digests of high-entropy tokens.
use crate::{Error, Result, bad, text};
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use subtle::ConstantTimeEq;
/// Operator-provided secrets are neither serializable nor Debug-printable.
pub struct Tokens {
    entries: Vec<Token>,
}
struct Token {
    digest: [u8; 32],
    subject: String,
    expires_at: Option<DateTime<Utc>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    subject: String,
    /// Lowercase hex SHA-256 of the bearer token.
    token_sha256: String,
    /// RFC 3339 expiry; expired entries are rejected without a restart.
    expires_at: Option<String>,
    /// Revoked entries stay in the file for audit but never authenticate.
    #[serde(default)]
    disabled: bool,
    /// Free-form operator label, e.g. the rotation generation.
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
}
pub fn sha256_hex(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn parse_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hex = std::str::from_utf8(chunk).ok()?;
        if hex.bytes().any(|b| b.is_ascii_uppercase()) {
            return None;
        }
        out[i] = u8::from_str_radix(hex, 16).ok()?;
    }
    Some(out)
}
impl Tokens {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 1024 * 1024 {
            return Err(bad());
        }
        let entries: Vec<Entry> =
            serde_json::from_slice(bytes).map_err(|_| Error::new(400, "invalid token file"))?;
        if entries.is_empty() || entries.len() > 1024 {
            return Err(bad());
        }
        let mut digests = BTreeSet::new();
        let mut out = Vec::new();
        for e in entries {
            text(&e.subject, 128)?;
            let digest = parse_hex(&e.token_sha256).ok_or_else(|| {
                Error::new(400, "token_sha256 must be 64 lowercase hex characters")
            })?;
            if !digests.insert(digest) {
                return Err(Error::new(400, "tokens must be unique"));
            }
            let expires_at = e
                .expires_at
                .as_deref()
                .map(|s| {
                    DateTime::parse_from_rfc3339(s)
                        .map(|t| t.with_timezone(&Utc))
                        .map_err(|_| Error::new(400, "expires_at must be RFC 3339"))
                })
                .transpose()?;
            if !e.disabled {
                out.push(Token {
                    digest,
                    subject: e.subject,
                    expires_at,
                });
            }
        }
        Ok(Self { entries: out })
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
        if token.is_empty() || token.len() > 256 {
            return None;
        }
        let presented: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let now = Utc::now();
        let mut subject = None;
        // Compare against every entry so timing does not reveal the matching position.
        for e in &self.entries {
            if bool::from(e.digest.ct_eq(&presented)) && e.expires_at.is_none_or(|t| t > now) {
                subject = Some(e.subject.clone());
            }
        }
        subject
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headers(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Ok(v) = format!("Bearer {token}").parse() {
            h.insert("authorization", v);
        }
        h
    }
    #[test]
    fn hashed_expired_and_disabled_entries() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let fresh = "a".repeat(43);
        let old = "b".repeat(43);
        let revoked = "c".repeat(43);
        let file = serde_json::json!([
            {"subject":"alice","token_sha256":sha256_hex(&fresh),"expires_at":"2999-01-01T00:00:00Z"},
            {"subject":"alice","token_sha256":sha256_hex(&old),"expires_at":"2001-01-01T00:00:00Z"},
            {"subject":"bob","token_sha256":sha256_hex(&revoked),"disabled":true},
        ]);
        let tokens = Tokens::from_json(&serde_json::to_vec(&file)?)?;
        assert_eq!(tokens.authenticate(&headers(&fresh)), Some("alice".into()));
        assert_eq!(tokens.authenticate(&headers(&old)), None);
        assert_eq!(tokens.authenticate(&headers(&revoked)), None);
        assert_eq!(tokens.authenticate(&headers("unknown")), None);
        for invalid in [
            serde_json::json!([{"subject":"x","token_sha256":"ABC"}]),
            serde_json::json!([{"subject":"x"}]),
            serde_json::json!([{"subject":"x","token":fresh}]),
            serde_json::json!([{"subject":"x","token_sha256":sha256_hex(&fresh),"expires_at":"tomorrow"}]),
            serde_json::json!([{"subject":"x","token_sha256":sha256_hex(&fresh)},{"subject":"y","token_sha256":sha256_hex(&fresh)}]),
        ] {
            assert!(
                Tokens::from_json(&serde_json::to_vec(&invalid)?).is_err(),
                "{invalid}"
            );
        }
        Ok(())
    }
}
