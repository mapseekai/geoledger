//! Global managed credentials. Only digests, metadata and non-secret receipts persist.
use crate::{Application, Error, Page, Policy, Result, Transaction, bad, id, missing, text};
use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Public metadata; deliberately contains neither digest nor plaintext.
#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedToken {
    pub id: String,
    pub subject: String,
    pub label: String,
    pub created_by: String,
    pub created_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
}
pub struct TokenReceipt {
    pub payload: String,
    pub token_id: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Issue {
    request_id: String,
    subject: String,
    label: String,
    expires_at: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reissue {
    request_id: String,
    token_id: String,
    label: String,
    expires_at: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TokenRef {
    token_id: String,
}
fn admin(subject: &str, policy: &Policy) -> Result<()> {
    if !policy.admins.contains(subject) {
        return Err(Error::new(403, "platform administrator required"));
    }
    Ok(())
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn digest(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn expiry(value: &str) -> Result<String> {
    let date = DateTime::parse_from_rfc3339(value)
        .map_err(|_| Error::new(400, "expires_at must be RFC 3339"))?
        .with_timezone(&Utc);
    if date <= Utc::now() || !(1..=9999).contains(&date.year()) {
        return Err(Error::new(
            400,
            "expiry must be in the future and before year 10000",
        ));
    }
    Ok(date.to_rfc3339_opts(SecondsFormat::Millis, true))
}
fn replay(t: &mut Transaction, actor: &str, request: &str, payload: &str) -> Result<Option<Value>> {
    if let Some(r) = t.token_receipt(actor, request)? {
        if r.payload != payload {
            return Err(Error::new(
                409,
                "request_id reused with different parameters",
            ));
        }
        let credential = t.managed_token(&r.token_id)?.ok_or_else(missing)?;
        return Ok(Some(json!({"credential":credential,"replayed":true})));
    }
    Ok(None)
}
fn generate(
    t: &mut Transaction,
    actor: &str,
    policy: &Policy,
    r: &Issue,
    payload: &str,
    replaces: Option<&str>,
) -> Result<Value> {
    text(&r.subject, 128)?;
    if policy.admins.contains(&r.subject) {
        return Err(Error::new(
            403,
            "cannot issue credentials for a platform administrator",
        ));
    }
    if r.label.len() > 256 || r.label.chars().any(char::is_control) {
        return Err(bad());
    }
    let expires_at = expiry(&r.expires_at)?;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::new(500, "credential generation failed"))?;
    let token = format!(
        "glmt_{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let credential = ManagedToken {
        id: Uuid::new_v4().to_string(),
        subject: r.subject.clone(),
        label: r.label.clone(),
        created_by: actor.into(),
        created_at: now(),
        expires_at,
        revoked_at: None,
    };
    t.insert_token(&credential, &digest(&token))?;
    if let Some(old) = replaces {
        t.revoke_token(old, &credential.created_at)?;
    }
    t.save_token_receipt(actor, &r.request_id, payload, &credential.id)?;
    t.append_token_audit(actor,if replaces.is_some(){"reissue_token"}else{"issue_token"},&credential.id,&json!({"subject":r.subject,"label":r.label,"created_at":credential.created_at,"expires_at":credential.expires_at,"replaces":replaces}).to_string())?;
    Ok(json!({"credential":credential,"token":token,"replayed":false}))
}
pub(super) fn issue(t: &mut Transaction, actor: &str, policy: &Policy, r: Issue) -> Result<Value> {
    admin(actor, policy)?;
    id(&r.request_id)?;
    let payload = json!({"operation":"issue_token","input":r}).to_string();
    t.lock_credentials()?;
    if let Some(result) = replay(t, actor, &r.request_id, &payload)? {
        return Ok(result);
    }
    generate(t, actor, policy, &r, &payload, None)
}
pub(super) fn reissue(
    t: &mut Transaction,
    actor: &str,
    policy: &Policy,
    r: Reissue,
) -> Result<Value> {
    admin(actor, policy)?;
    id(&r.request_id)?;
    id(&r.token_id)?;
    let payload = json!({"operation":"reissue_token","input":r}).to_string();
    t.lock_credentials()?;
    if let Some(result) = replay(t, actor, &r.request_id, &payload)? {
        return Ok(result);
    }
    let old = t.managed_token(&r.token_id)?.ok_or_else(missing)?;
    if old.revoked_at.is_some() {
        return Err(Error::new(
            409,
            "credential already revoked; issue a new token",
        ));
    }
    let input = Issue {
        request_id: r.request_id,
        subject: old.subject,
        label: r.label,
        expires_at: r.expires_at,
    };
    generate(t, actor, policy, &input, &payload, Some(&r.token_id))
}
pub(super) fn revoke(
    t: &mut Transaction,
    actor: &str,
    policy: &Policy,
    r: TokenRef,
) -> Result<Value> {
    admin(actor, policy)?;
    id(&r.token_id)?;
    t.lock_credentials()?;
    let old = t.managed_token(&r.token_id)?.ok_or_else(missing)?;
    if old.revoked_at.is_none() {
        let time = now();
        t.revoke_token(&r.token_id, &time)?;
        t.append_token_audit(
            actor,
            "revoke_token",
            &r.token_id,
            &json!({"subject":old.subject,"label":old.label,"revoked_at":time}).to_string(),
        )?;
    }
    Ok(json!({"ok":true}))
}
pub(super) fn list(t: &mut Transaction, actor: &str, policy: &Policy, r: Page) -> Result<Value> {
    admin(actor, policy)?;
    r.check()?;
    serde_json::to_value(t.list_tokens(&r.after, r.limit)?).map_err(Error::invalid_json)
}
impl Application {
    /// No positive cache: committed revocations are observed by subsequent admissions.
    pub fn authenticate_managed_token(&self, token: &str) -> Result<Option<String>> {
        if token.len() != 69
            || !token.starts_with("glmt_")
            || !token[5..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Ok(None);
        }
        let mut t = self.storage.begin(true, self.timeout)?;
        let subject = t.managed_subject(&digest(token), &now())?;
        t.commit()?;
        // Configuration changes must not turn an existing managed identity into an admin.
        Ok(subject.filter(|s| !self.policy.admins.contains(s)))
    }
}
