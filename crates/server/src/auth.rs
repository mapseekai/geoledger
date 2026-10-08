//! Offline validation of issuer-managed RS256 access tokens against trusted JWKS.
//! Key distribution/rotation is an operator responsibility; token headers never
//! select a URL, algorithm policy, issuer, or audience.
use crate::{Error, Result, Tokens, text};
use axum::http::HeaderMap;
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse},
};
use serde::Deserialize;
use std::collections::BTreeMap;

pub enum Authentication {
    Tokens(Tokens),
    Jwt(Box<JwtAuthenticator>),
}
impl Authentication {
    pub(crate) fn authenticate(&self, headers: &HeaderMap) -> Option<String> {
        match self {
            Self::Tokens(tokens) => tokens.authenticate(headers),
            Self::Jwt(jwt) => {
                if headers.get_all("authorization").iter().count() != 1 {
                    return None;
                }
                jwt.authenticate(
                    headers
                        .get("authorization")?
                        .to_str()
                        .ok()?
                        .strip_prefix("Bearer ")?,
                )
            }
        }
    }
}
/// Validates signature, fixed issuer/audience, expiry, optional nbf and subject.
/// No Debug implementation: authentication material is never logged.
pub struct JwtAuthenticator {
    keys: BTreeMap<String, DecodingKey>,
    validation: Validation,
}
#[derive(Deserialize)]
struct Claims {
    sub: String,
}
impl JwtAuthenticator {
    pub fn from_jwks(bytes: &[u8], issuer: &str, audience: &str) -> Result<Self> {
        text(issuer, 2048)?;
        text(audience, 2048)?;
        if !issuer.starts_with("https://") || bytes.len() > 1024 * 1024 {
            return Err(Error::new(
                400,
                "JWT requires an HTTPS issuer and a bounded trusted JWKS",
            ));
        }
        let jwks: JwkSet =
            serde_json::from_slice(bytes).map_err(|_| Error::new(400, "invalid JWKS"))?;
        if jwks.keys.is_empty() || jwks.keys.len() > 64 {
            return Err(Error::new(400, "invalid JWKS key count"));
        }
        let mut keys = BTreeMap::new();
        for jwk in jwks.keys {
            if !matches!(jwk.algorithm, AlgorithmParameters::RSA(_))
                || jwk
                    .common
                    .key_algorithm
                    .is_some_and(|a| a != KeyAlgorithm::RS256)
                || jwk
                    .common
                    .public_key_use
                    .as_ref()
                    .is_some_and(|u| *u != PublicKeyUse::Signature)
                || jwk
                    .common
                    .key_operations
                    .as_ref()
                    .is_some_and(|ops| !ops.contains(&KeyOperations::Verify))
            {
                return Err(Error::new(
                    400,
                    "JWKS must contain RS256 signature verification keys",
                ));
            }
            let kid = jwk
                .common
                .key_id
                .as_deref()
                .ok_or_else(|| Error::new(400, "JWKS key requires kid"))?;
            text(kid, 256)?;
            let key =
                DecodingKey::from_jwk(&jwk).map_err(|_| Error::new(400, "invalid RSA JWK"))?;
            if keys.insert(kid.to_owned(), key).is_some() {
                return Err(Error::new(400, "duplicate JWKS kid"));
            }
        }
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[issuer]);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.validate_nbf = true;
        validation.leeway = 30;
        Ok(Self { keys, validation })
    }
    fn authenticate(&self, token: &str) -> Option<String> {
        if token.len() > 16384 {
            return None;
        }
        let header = decode_header(token).ok()?;
        if header.alg != Algorithm::RS256 {
            return None;
        }
        let key = self.keys.get(header.kid.as_deref()?)?;
        let claims = decode::<Claims>(token, key, &self.validation).ok()?.claims;
        text(&claims.sub, 128).ok()?;
        Some(claims.sub)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::{Value, json};
    fn signed(
        claims: &Value,
        kid: &str,
    ) -> std::result::Result<String, Box<dyn std::error::Error>> {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.into());
        // This publicly committed fixture is exclusively for tests; never a deployment key.
        Ok(encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/jwt-test-only.pem"))?,
        )?)
    }
    #[test]
    fn jwt_verifies_signature_issuer_audience_time_key_and_subject()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let auth = JwtAuthenticator::from_jwks(
            include_bytes!("../tests/fixtures/jwks.json"),
            "https://issuer.example",
            "geoledger",
        )?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let claims = json!({"iss":"https://issuer.example","aud":"geoledger","sub":"alice","exp":now+300,"nbf":now});
        assert_eq!(
            auth.authenticate(&signed(&claims, "test-only")?),
            Some("alice".into())
        );
        for (field, value) in [
            ("iss", json!("https://other.example")),
            ("aud", json!("other")),
            ("sub", json!("")),
            ("exp", json!(now - 120)),
            ("nbf", json!(now + 120)),
        ] {
            let mut invalid = claims.clone();
            invalid[field] = value;
            assert!(
                auth.authenticate(&signed(&invalid, "test-only")?).is_none(),
                "{field}"
            );
        }
        for field in ["exp", "iss", "aud", "sub"] {
            let mut invalid = claims.clone();
            invalid.as_object_mut().ok_or("claims")?.remove(field);
            assert!(
                auth.authenticate(&signed(&invalid, "test-only")?).is_none(),
                "missing {field}"
            );
        }
        assert!(auth.authenticate(&signed(&claims, "unknown")?).is_none());
        let mut token = signed(&claims, "test-only")?;
        let index = token.rfind('.').ok_or("signature")? + 1;
        token.replace_range(
            index..index + 1,
            if &token[index..index + 1] == "A" {
                "B"
            } else {
                "A"
            },
        );
        assert!(auth.authenticate(&token).is_none());
        let mut header = Header::new(Algorithm::HS256);
        header.kid = Some("test-only".into());
        let token = encode(&header, &claims, &EncodingKey::from_secret(b"untrusted"))?;
        assert!(auth.authenticate(&token).is_none());
        Ok(())
    }
    #[test]
    fn jwks_rejects_duplicate_keys_and_unsupported_policy()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let bytes = include_bytes!("../tests/fixtures/jwks.json");
        let mut value: Value = serde_json::from_slice(bytes)?;
        let key = value["keys"][0].clone();
        value["keys"].as_array_mut().ok_or("keys")?.push(key);
        assert!(
            JwtAuthenticator::from_jwks(
                &serde_json::to_vec(&value)?,
                "https://issuer.example",
                "geoledger"
            )
            .is_err()
        );
        assert!(JwtAuthenticator::from_jwks(bytes, "http://issuer.example", "geoledger").is_err());
        assert!(JwtAuthenticator::from_jwks(bytes, "https://issuer.example", "").is_err());
        Ok(())
    }
}
