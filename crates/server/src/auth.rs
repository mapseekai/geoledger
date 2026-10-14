//! Strict transport credential parsing. Identity and authorization live in the engine.
use crate::{Error, Result};
use axum::http::HeaderMap;
pub(crate) enum Credential {
    ApiKey(String),
    Session(String),
}
pub(crate) fn credential(headers: &HeaderMap, allow_session: bool) -> Result<Credential> {
    let auth = headers.get_all("authorization").iter().count();
    let sessions = headers.get_all("x-geoledger-session").iter().count();
    let invalid = || Error::new(401, "authentication failed");
    if auth + sessions != 1 {
        return Err(invalid());
    }
    if auth == 1 {
        let key = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(invalid)?;
        if key.len() != 68
            || !key.starts_with("glk_")
            || !key[4..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        Ok(Credential::ApiKey(key.to_owned()))
    } else if allow_session {
        let value = headers
            .get("x-geoledger-session")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(invalid)?;
        if value.is_empty()
            || value.len() > 256
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(invalid());
        }
        Ok(Credential::Session(value.to_owned()))
    } else {
        Err(invalid())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_credentials_and_sessions_on_grpc()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer glk_{}", "a".repeat(64)).parse()?,
        );
        assert!(credential(&headers, false).is_ok());
        headers.insert(
            "authorization",
            format!("Bearer glk_{}", "A".repeat(64)).parse()?,
        );
        assert!(credential(&headers, false).is_err());
        headers.insert(
            "authorization",
            format!("Bearer glk_{}", "a".repeat(64)).parse()?,
        );
        headers.append("authorization", headers["authorization"].clone());
        assert!(credential(&headers, true).is_err());
        headers.remove("authorization");
        headers.insert("x-geoledger-session", "gls_test".parse()?);
        assert!(credential(&headers, true).is_ok());
        assert!(credential(&headers, false).is_err());
        headers.insert(
            "authorization",
            format!("Bearer glk_{}", "a".repeat(64)).parse()?,
        );
        assert!(credential(&headers, true).is_err());
        Ok(())
    }
}
