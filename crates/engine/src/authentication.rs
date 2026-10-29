//! Database identities, password verification and opaque credentials. No secret is serializable.
use crate::repository::CredentialRepository;
use crate::{Application, Error, Result, bad, missing, text};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use uuid::Uuid;

struct Deadline(Instant);
impl Deadline {
    fn new(timeout: Duration) -> Result<Self> {
        clock()
            .checked_add(timeout)
            .map(Self)
            .ok_or_else(|| Error::new(400, "invalid operation deadline"))
    }
    fn remaining(&self) -> Result<Duration> {
        let remaining = self.0.saturating_duration_since(clock());
        if remaining.is_zero() {
            Err(Error::new(504, "operation deadline exceeded"))
        } else {
            Ok(remaining)
        }
    }
}
fn clock() -> Instant {
    #[cfg(test)]
    {
        TEST_CLOCK.with(|clock| clock.get().unwrap_or_else(Instant::now))
    }
    #[cfg(not(test))]
    Instant::now()
}
#[cfg(test)]
thread_local! { static TEST_CLOCK: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) }; }

#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub status: String,
    pub is_admin: bool,
    pub must_change_password: bool,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub user_id: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub last_used_at: Option<String>,
}
/// Storage-only record: deliberately neither Debug nor Serialize.
pub struct UserRecord {
    pub user: User,
    pub password_hash: String,
    pub credential_version: i64,
}
pub struct SessionRecord {
    pub id: String,
    pub user_id: String,
    pub source_key_id: Option<String>,
    pub credential_version: i64,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
}
#[derive(Clone)]
enum Proof {
    ApiKey(String),
    Session(String),
}
/// Constructed only after checking the database. Credentials and digests are never formatted.
#[derive(Clone)]
pub struct AuthenticatedIdentity {
    pub user: User,
    pub session_id: Option<String>,
    pub source_api_key: Option<String>,
    proof: Proof,
}
impl std::fmt::Debug for AuthenticatedIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthenticatedIdentity")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn ago(minutes: i64) -> String {
    (Utc::now() - chrono::Duration::minutes(minutes)).to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn unauthenticated() -> Error {
    Error::new(401, "authentication failed")
}
fn forbidden() -> Error {
    Error::new(403, "permission denied")
}
fn digest(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn secret(prefix: &str) -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::new(500, "credential generation failed"))?;
    Ok(format!(
        "{prefix}{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}
fn valid_secret(value: &str, prefix: &str) -> bool {
    value.len() == prefix.len() + 64
        && value.starts_with(prefix)
        && value[prefix.len()..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn username(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    if !(3..=64).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::new(
            400,
            "username must contain 3 to 64 ASCII letters, digits, dots, underscores or hyphens",
        ));
    }
    Ok(value)
}
fn password(value: &str) -> Result<()> {
    if !(12..=1024).contains(&value.len()) {
        Err(Error::new(400, "password must contain 12 to 1024 bytes"))
    } else {
        Ok(())
    }
}
fn hash_password(value: &str) -> Result<String> {
    password(value)?;
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| Error::new(500, "password salt generation failed"))?;
    let salt = SaltString::encode_b64(&salt)
        .map_err(|_| Error::new(500, "password salt generation failed"))?;
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| Error::new(500, "password hashing failed"))
}
fn verify(value: &str, hash: &str) -> bool {
    value.len() <= 1024
        && PasswordHash::new(hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(value.as_bytes(), &h)
                .is_ok()
        })
}
fn dummy_hash() -> Result<&'static str> {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(hash) = HASH.get() {
        return Ok(hash);
    }
    let hash = hash_password(&secret("dummy_")?)?;
    let _ = HASH.set(hash);
    HASH.get()
        .map(String::as_str)
        .ok_or_else(|| Error::new(500, "password verification unavailable"))
}
fn string<'a>(input: &'a Value, field: &str) -> Result<&'a str> {
    input.get(field).and_then(Value::as_str).ok_or_else(bad)
}
fn optional_text(input: &Value, field: &str, max: usize) -> Result<Option<String>> {
    match input.get(field) {
        None => Ok(None),
        Some(Value::String(s)) => {
            if s.len() > max || s.chars().any(char::is_control) {
                return Err(bad());
            }
            Ok(Some(s.clone()))
        }
        _ => Err(bad()),
    }
}
fn expiry(input: &Value) -> Result<Option<String>> {
    let Some(v) = input.get("expires_at").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let date = DateTime::parse_from_rfc3339(v.as_str().ok_or_else(bad)?)
        .map_err(|_| Error::new(400, "expires_at must be RFC 3339"))?
        .with_timezone(&Utc);
    if date <= Utc::now() || !(1..=9999).contains(&date.year()) {
        return Err(Error::new(
            400,
            "expiry must be in the future and before year 10000",
        ));
    }
    Ok(Some(date.to_rfc3339_opts(SecondsFormat::Millis, true)))
}
fn page(input: &Value) -> Result<(&str, i64)> {
    let after = input
        .get("after")
        .map_or(Ok(""), |v| v.as_str().ok_or_else(bad))?;
    let limit = input
        .get("limit")
        .map_or(Ok(50), |v| v.as_i64().ok_or_else(bad))?;
    if after.len() > 128 || !(1..=100).contains(&limit) {
        return Err(bad());
    }
    Ok((after, limit))
}
fn active(t: &mut (impl CredentialRepository + ?Sized), id: &str) -> Result<UserRecord> {
    t.auth_user(id)?
        .filter(|u| u.user.status == "active")
        .ok_or_else(unauthenticated)
}
fn valid_key(key: &ApiKey, time: &str) -> bool {
    key.revoked_at.is_none() && key.expires_at.as_deref().is_none_or(|e| e > time)
}
fn resolve(
    t: &mut (impl CredentialRepository + ?Sized),
    proof: &Proof,
) -> Result<(UserRecord, Option<SessionRecord>, Option<ApiKey>)> {
    let time = now();
    match proof {
        Proof::ApiKey(d) => {
            let key = t.auth_key_digest(d, &time)?.ok_or_else(unauthenticated)?;
            let user = active(t, &key.user_id)?;
            if user.user.must_change_password {
                return Err(Error::new(403, "password change required"));
            }
            Ok((user, None, Some(key)))
        }
        Proof::Session(d) => {
            let session = t
                .auth_session(d)?
                .filter(|s| s.expires_at > time && s.last_seen_at > ago(30))
                .ok_or_else(unauthenticated)?;
            let user = active(t, &session.user_id)?;
            if user.credential_version != session.credential_version {
                return Err(unauthenticated());
            }
            let key = if let Some(id) = &session.source_key_id {
                Some(
                    t.auth_key(id)?
                        .filter(|k| valid_key(k, &time))
                        .ok_or_else(unauthenticated)?,
                )
            } else {
                None
            };
            if key.is_some() && user.user.must_change_password {
                return Err(Error::new(403, "password change required"));
            }
            Ok((user, Some(session), key))
        }
    }
}
pub(crate) fn business_identity(
    t: &mut (impl CredentialRepository + ?Sized),
    identity: &AuthenticatedIdentity,
    verified_password: Option<&UserRecord>,
) -> Result<User> {
    let (actor, _, _) = resolve(t, &identity.proof)?;
    if actor.user.must_change_password {
        return Err(Error::new(403, "password change required"));
    }
    if let Some(verified) = verified_password
        && (actor.credential_version != verified.credential_version
            || actor.password_hash != verified.password_hash)
    {
        return Err(unauthenticated());
    }
    Ok(actor.user)
}
fn new_session(
    t: &mut (impl CredentialRepository + ?Sized),
    user: &UserRecord,
    key: Option<String>,
) -> Result<Value> {
    let token = secret("gls_")?;
    let time = now();
    let session = SessionRecord {
        id: Uuid::new_v4().to_string(),
        user_id: user.user.id.clone(),
        source_key_id: key,
        credential_version: user.credential_version,
        created_at: time.clone(),
        last_seen_at: time,
        expires_at: (Utc::now() + chrono::Duration::hours(12))
            .to_rfc3339_opts(SecondsFormat::Millis, true),
    };
    t.auth_insert_session(&session, &digest(&token))?;
    Ok(json!({"session":token,"user":user.user}))
}
fn scoped_user(actor: &User, id: &str) -> Result<()> {
    if actor.id != id && !actor.is_admin {
        Err(forbidden())
    } else {
        Ok(())
    }
}
impl Application {
    /// Public transport entry point accepts only a database-authenticated identity.
    pub fn execute_authenticated(
        &self,
        identity: &AuthenticatedIdentity,
        operation: &str,
        mut input: Value,
    ) -> Result<Value> {
        let started = std::time::Instant::now();
        let verified = if matches!(identity.proof, Proof::Session(_))
            && matches!(operation, "set_member" | "remove_member")
        {
            let mut t = self.storage.begin(true, self.timeout)?;
            let (actor, _, _) = resolve(t.as_mut(), &identity.proof)?;
            t.commit()?;
            let supplied = input
                .get("current_password")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !verify(supplied, &actor.password_hash) {
                return Err(Error::new(403, "current password verification required"));
            }
            input
                .as_object_mut()
                .ok_or_else(bad)?
                .remove("current_password");
            Some(actor)
        } else {
            None
        };
        let remaining = self.timeout.saturating_sub(started.elapsed());
        self.clone()
            .with_timeout(remaining)
            .execute_encoded(
                &identity.user.id,
                operation,
                input,
                Some(identity),
                verified.as_ref(),
            )
            .map(|(value, _)| value)
    }

    pub fn bootstrap_needed(&self) -> Result<bool> {
        let deadline = Deadline::new(self.timeout)?;
        let mut t = self.storage.begin(true, deadline.remaining()?)?;
        let needed = t.auth_user_count()? == 0;
        deadline.remaining()?;
        t.commit()?;
        Ok(needed)
    }
    /// Hashing precedes transaction admission; lock serializes competing bootstrap attempts.
    pub fn bootstrap_admin(&self, name: &str, value: &str) -> Result<bool> {
        let deadline = Deadline::new(self.timeout)?;
        let mut t = self.storage.begin(true, deadline.remaining()?)?;
        let needed = t.auth_user_count()? == 0;
        deadline.remaining()?;
        t.commit()?;
        if !needed {
            return Ok(false);
        }
        let name = username(name)?;
        let hash = hash_password(value)?;
        let mut t = self.storage.begin(false, deadline.remaining()?)?;
        t.lock_credentials()?;
        if t.auth_user_count()? != 0 {
            deadline.remaining()?;
            t.commit()?;
            return Ok(false);
        }
        let user = UserRecord {
            user: User {
                id: Uuid::new_v4().to_string(),
                username: name,
                display_name: "Administrator".into(),
                status: "active".into(),
                is_admin: true,
                must_change_password: true,
                created_at: now(),
            },
            password_hash: hash,
            credential_version: 0,
        };
        t.auth_insert_user(&user)?;
        t.auth_audit(&user.user.id, "bootstrap_admin", &user.user.id, &now())?;
        deadline.remaining()?;
        t.commit()?;
        Ok(true)
    }
    fn authenticate(&self, proof: Proof) -> Result<AuthenticatedIdentity> {
        let deadline = Deadline::new(self.timeout)?;
        let mut t = self.storage.begin(true, deadline.remaining()?)?;
        let (user, session, key) = resolve(t.as_mut(), &proof)?;
        deadline.remaining()?;
        t.commit()?;
        let session_touch = session.as_ref().filter(|s| s.last_seen_at < ago(1));
        let key_touch = key.as_ref().filter(|k| {
            k.last_used_at
                .as_deref()
                .is_none_or(|v| v < ago(5).as_str())
        });
        if session_touch.is_some() || key_touch.is_some() {
            let mut t = self.storage.begin(false, deadline.remaining()?)?;
            // A concurrent revocation must not be undone by activity bookkeeping.
            resolve(t.as_mut(), &proof)?;
            // Key deletion cascades to sessions, so every writer locks keys first.
            if let Some(k) = key_touch {
                t.auth_touch_key(&k.id, &now(), &ago(5))?;
            }
            if let Some(s) = session_touch {
                t.auth_touch_session(&s.id, &now(), &ago(1))?;
            }
            deadline.remaining()?;
            t.commit()?;
        }
        deadline.remaining()?;
        Ok(AuthenticatedIdentity {
            user: user.user,
            session_id: session.map(|s| s.id),
            source_api_key: key.map(|k| k.id),
            proof,
        })
    }
    pub fn authenticate_api_key(&self, key: &str) -> Result<AuthenticatedIdentity> {
        if !valid_secret(key, "glk_") {
            return Err(unauthenticated());
        }
        self.authenticate(Proof::ApiKey(digest(key)))
    }
    pub fn authenticate_session(&self, session: &str) -> Result<AuthenticatedIdentity> {
        if !valid_secret(session, "gls_") {
            return Err(unauthenticated());
        }
        self.authenticate(Proof::Session(digest(session)))
    }
    pub fn auth_operation(
        &self,
        identity: Option<&AuthenticatedIdentity>,
        operation: &str,
        input: Value,
    ) -> Result<Value> {
        let deadline = Deadline::new(self.timeout)?;
        if !input.is_object() {
            return Err(bad());
        }
        if operation == "login_password" {
            let name = string(&input, "username")?;
            let supplied = string(&input, "password")?;
            let mut t = self.storage.begin(true, deadline.remaining()?)?;
            let record = match username(name) {
                Ok(name) => t.auth_username(&name)?,
                Err(_) => None,
            };
            deadline.remaining()?;
            t.commit()?;
            let hash = record
                .as_ref()
                .filter(|u| u.user.status == "active")
                .map(|u| u.password_hash.as_str())
                .unwrap_or(dummy_hash()?);
            let correct = verify(supplied, hash);
            deadline.remaining()?;
            let record = record
                .filter(|u| u.user.status == "active" && correct)
                .ok_or_else(unauthenticated)?;
            let mut t = self.storage.begin(false, deadline.remaining()?)?;
            t.lock_credentials()?;
            let current = active(t.as_mut(), &record.user.id)?;
            if current.password_hash != record.password_hash
                || current.credential_version != record.credential_version
            {
                return Err(unauthenticated());
            }
            let result = new_session(t.as_mut(), &current, None)?;
            t.auth_audit(&current.user.id, "login_password", &current.user.id, &now())?;
            crate::codec::encode_limited(
                &result,
                self.response_limit,
                self.response_reservation.clone(),
            )?;
            deadline.remaining()?;
            t.commit()?;
            return Ok(result);
        }
        if operation == "login_api_key" {
            let key = string(&input, "api_key")?;
            if !valid_secret(key, "glk_") {
                return Err(unauthenticated());
            }
            let mut t = self.storage.begin(false, deadline.remaining()?)?;
            t.lock_credentials()?;
            let (user, _, key) = resolve(t.as_mut(), &Proof::ApiKey(digest(key)))?;
            let result = new_session(t.as_mut(), &user, key.map(|k| k.id))?;
            t.auth_audit(&user.user.id, "login_api_key", &user.user.id, &now())?;
            crate::codec::encode_limited(
                &result,
                self.response_limit,
                self.response_reservation.clone(),
            )?;
            deadline.remaining()?;
            t.commit()?;
            return Ok(result);
        }
        let identity = identity.ok_or_else(unauthenticated)?;
        let read_only = matches!(operation, "me" | "list_users" | "list_api_keys");
        let sensitive = !matches!(operation, "me" | "logout" | "list_users" | "list_api_keys");
        let mut initial = self.storage.begin(true, deadline.remaining()?)?;
        let (actor, session, _) = resolve(initial.as_mut(), &identity.proof)?;
        let read = if read_only {
            Some(initial)
        } else {
            deadline.remaining()?;
            initial.commit()?;
            None
        };
        if actor.user.must_change_password
            && !matches!(operation, "me" | "logout" | "change_password")
        {
            return Err(Error::new(403, "password change required"));
        }
        if sensitive {
            let correct = verify(
                input
                    .get("current_password")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                &actor.password_hash,
            );
            deadline.remaining()?;
            if !correct {
                return Err(Error::new(403, "current password verification required"));
            }
        }
        // Password work never holds the global mutation lock or a database transaction.
        let password_hash = match operation {
            "create_user" => Some(hash_password(string(&input, "password")?)?),
            "change_password" | "reset_password" => {
                Some(hash_password(string(&input, "new_password")?)?)
            }
            _ => None,
        };
        if operation == "change_password"
            && string(&input, "new_password")? == string(&input, "current_password")?
        {
            return Err(Error::new(
                400,
                "new password must differ from current password",
            ));
        }
        deadline.remaining()?;
        let (mut t, actor, session) = if let Some(t) = read {
            (t, actor, session)
        } else {
            let mut t = self.storage.begin(false, deadline.remaining()?)?;
            t.lock_credentials()?;
            let (current, session, _) = resolve(t.as_mut(), &identity.proof)?;
            if current.credential_version != actor.credential_version
                || current.password_hash != actor.password_hash
            {
                return Err(unauthenticated());
            }
            (t, current, session)
        };
        t.set_response_limit(self.response_limit);
        t.set_response_reservation(self.response_reservation.clone());
        let time = now();
        let mut resource = actor.user.id.clone();
        let result = match operation {
            "me" => json!(actor.user),
            "logout" => {
                let s = session.ok_or_else(forbidden)?;
                t.auth_delete_session(&s.id)?;
                json!({"ok":true})
            }
            "list_users" => {
                if !actor.user.is_admin {
                    return Err(forbidden());
                }
                let (after, limit) = page(&input)?;
                let status = input
                    .get("status")
                    .map_or(Ok(""), |v| v.as_str().ok_or_else(bad))?;
                if !["", "active", "disabled", "deleted"].contains(&status) {
                    return Err(bad());
                }
                json!(t.auth_users(after, limit, status)?)
            }
            "create_user" => {
                if !actor.user.is_admin {
                    return Err(forbidden());
                }
                let user = UserRecord {
                    user: User {
                        id: Uuid::new_v4().to_string(),
                        username: username(string(&input, "username")?)?,
                        display_name: optional_text(&input, "display_name", 256)?
                            .unwrap_or_default(),
                        status: "active".into(),
                        is_admin: input
                            .get("is_admin")
                            .map_or(Ok(false), |v| v.as_bool().ok_or_else(bad))?,
                        must_change_password: true,
                        created_at: time.clone(),
                    },
                    password_hash: password_hash.ok_or_else(bad)?,
                    credential_version: 0,
                };
                resource = user.user.id.clone();
                t.auth_insert_user(&user)?;
                json!(user.user)
            }
            "update_user" => {
                let target = string(&input, "user_id")?;
                scoped_user(&actor.user, target)?;
                let mut user = t.auth_user(target)?.ok_or_else(missing)?;
                if user.user.status == "deleted" {
                    return Err(Error::new(409, "deleted identity cannot be restored"));
                }
                if let Some(name) = input.get("username") {
                    user.user.username = username(name.as_str().ok_or_else(bad)?)?;
                }
                if let Some(name) = optional_text(&input, "display_name", 256)? {
                    user.user.display_name = name;
                }
                let was_admin = user.user.is_admin && user.user.status == "active";
                let mut invalidate = false;
                if let Some(status) = input.get("status") {
                    if !actor.user.is_admin {
                        return Err(forbidden());
                    }
                    let status = status.as_str().ok_or_else(bad)?;
                    if !["active", "disabled", "deleted"].contains(&status) {
                        return Err(bad());
                    }
                    invalidate |= user.user.status != status;
                    user.user.status = status.into();
                }
                if let Some(admin) = input.get("is_admin") {
                    if !actor.user.is_admin {
                        return Err(forbidden());
                    }
                    let admin = admin.as_bool().ok_or_else(bad)?;
                    invalidate |= user.user.is_admin != admin;
                    user.user.is_admin = admin;
                }
                if was_admin
                    && !(user.user.is_admin && user.user.status == "active")
                    && t.auth_admin_count()? <= 1
                {
                    return Err(Error::new(
                        409,
                        "last active administrator cannot be removed",
                    ));
                }
                if invalidate {
                    user.credential_version += 1;
                    t.auth_invalidate_user(target, &time, user.user.status != "active")?;
                }
                t.auth_update_user(&user)?;
                resource = target.into();
                json!(user.user)
            }
            "change_password" | "reset_password" => {
                let target = if operation == "reset_password" {
                    if !actor.user.is_admin {
                        return Err(forbidden());
                    }
                    string(&input, "user_id")?
                } else {
                    &actor.user.id
                };
                let mut user = t
                    .auth_user(target)?
                    .filter(|u| u.user.status != "deleted")
                    .ok_or_else(missing)?;
                user.password_hash = password_hash.ok_or_else(bad)?;
                user.credential_version += 1;
                user.user.must_change_password = operation == "reset_password";
                t.auth_update_user(&user)?;
                t.auth_invalidate_user(target, &time, true)?;
                resource = target.into();
                if operation == "change_password" {
                    new_session(t.as_mut(), &user, None)?
                } else {
                    json!({"ok":true})
                }
            }
            "list_api_keys" | "create_api_key" => {
                let target = input
                    .get("user_id")
                    .map_or(Ok(actor.user.id.as_str()), |v| v.as_str().ok_or_else(bad))?;
                scoped_user(&actor.user, target)?;
                if operation == "list_api_keys" {
                    let (after, limit) = page(&input)?;
                    json!(t.auth_keys(target, after, limit)?)
                } else {
                    let user = active(t.as_mut(), target)?;
                    if user.user.must_change_password {
                        return Err(Error::new(403, "password change required"));
                    }
                    let name = string(&input, "name")?;
                    text(name, 256)?;
                    let api_key = secret("glk_")?;
                    let key = ApiKey {
                        id: Uuid::new_v4().to_string(),
                        name: name.into(),
                        key_prefix: api_key[..12].into(),
                        user_id: target.into(),
                        created_at: time.clone(),
                        expires_at: expiry(&input)?,
                        revoked_at: None,
                        last_used_at: None,
                    };
                    t.auth_insert_key(&key, &digest(&api_key))?;
                    resource = key.id.clone();
                    json!({"api_key":api_key,"credential":key})
                }
            }
            "update_api_key" | "revoke_api_key" | "delete_api_key" => {
                let id = string(&input, "key_id")?;
                let mut key = t.auth_key(id)?.ok_or_else(missing)?;
                scoped_user(&actor.user, &key.user_id)?;
                resource = id.into();
                match operation {
                    "delete_api_key" => {
                        t.auth_delete_key(id)?;
                        json!({"ok":true})
                    }
                    "revoke_api_key" => {
                        if key.revoked_at.is_none() {
                            key.revoked_at = Some(time.clone());
                            t.auth_update_key(&key)?;
                        }
                        json!({"ok":true})
                    }
                    _ => {
                        if let Some(name) = optional_text(&input, "name", 256)? {
                            text(&name, 256)?;
                            key.name = name;
                        }
                        if input.get("expires_at").is_some() {
                            key.expires_at = expiry(&input)?;
                        }
                        t.auth_update_key(&key)?;
                        json!(key)
                    }
                }
            }
            _ => return Err(Error::new(400, "unknown authentication operation")),
        };
        if !read_only {
            t.auth_audit(&actor.user.id, operation, &resource, &time)?;
        }
        crate::codec::encode_limited(
            &result,
            self.response_limit,
            self.response_reservation.clone(),
        )?;
        deadline.remaining()?;
        t.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Storage, StorageBackend, Transaction};
    use std::sync::{Arc, Mutex};
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    struct Clock;
    impl Clock {
        fn start() -> Self {
            TEST_CLOCK.with(|c| c.set(Some(Instant::now())));
            Self
        }
        fn advance(duration: Duration) {
            TEST_CLOCK.with(|c| c.set(Some(c.get().unwrap_or_else(Instant::now) + duration)));
        }
    }
    impl Drop for Clock {
        fn drop(&mut self) {
            TEST_CLOCK.with(|c| c.set(None));
        }
    }
    struct TimedStorage {
        inner: Arc<dyn StorageBackend>,
        elapsed: Duration,
        admissions: Mutex<Vec<(bool, Duration)>>,
    }
    impl crate::repository::StorageMaintenance for TimedStorage {}
    impl StorageBackend for TimedStorage {
        fn name(&self) -> &'static str {
            self.inner.name()
        }
        fn initialize(&self, timeout: Duration) -> Result<()> {
            self.inner.initialize(timeout)
        }
        fn health(&self, timeout: Duration) -> Result<()> {
            self.inner.health(timeout)
        }
        fn begin(&self, read: bool, timeout: Duration) -> Result<Transaction> {
            self.admissions
                .lock()
                .map_err(|_| Error::new(500, "test mutex poisoned"))?
                .push((read, timeout));
            let tx = self.inner.begin(read, timeout)?;
            // Deterministic elapsed work; real SQL transactions still roll back on Drop.
            Clock::advance(self.elapsed);
            Ok(tx)
        }
    }
    fn timed(app: &Application, millis: u64) -> (Application, Arc<TimedStorage>) {
        let storage = Arc::new(TimedStorage {
            inner: app.storage.clone(),
            elapsed: Duration::from_millis(millis),
            admissions: Mutex::new(vec![]),
        });
        (
            Application::with_backend(storage.clone()).with_timeout(Duration::from_secs(1)),
            storage,
        )
    }
    fn fixture() -> std::result::Result<
        (tempfile::TempDir, Application, rusqlite::Connection),
        Box<dyn std::error::Error>,
    > {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("deadlines.db");
        let app = Application::new(Storage::Sqlite(path.clone()));
        app.migrate()?;
        let db = rusqlite::Connection::open(path)?;
        Ok((dir, app, db))
    }
    fn snapshot(db: &rusqlite::Connection) -> std::result::Result<Vec<i64>, rusqlite::Error> {
        [
            "gl_users",
            "gl_passwords",
            "gl_sessions",
            "gl_api_keys",
            "gl_auth_audit",
        ]
        .iter()
        .map(|table| db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)))
        .collect()
    }
    #[test]
    fn bootstrap_and_login_share_one_deadline_and_roll_back() -> TestResult {
        let (_dir, app, db) = fixture()?;
        let (limited, storage) = timed(&app, 600);
        {
            let _clock = Clock::start();
            assert_eq!(
                limited
                    .bootstrap_admin("admin", "test-password-123")
                    .err()
                    .ok_or("expected timeout")?
                    .status,
                504
            );
        }
        assert_eq!(snapshot(&db)?, vec![0; 5]);
        assert_eq!(
            *storage
                .admissions
                .lock()
                .map_err(|_| Error::new(500, "test mutex poisoned"))?,
            vec![
                (true, Duration::from_millis(1000)),
                (false, Duration::from_millis(400))
            ]
        );
        app.bootstrap_admin("admin", "test-password-123")?;
        let before = snapshot(&db)?;
        let (limited, storage) = timed(&app, 600);
        {
            let _clock = Clock::start();
            assert_eq!(
                limited
                    .auth_operation(
                        None,
                        "login_password",
                        json!({"username":"admin", "password":"test-password-123"})
                    )
                    .err()
                    .ok_or("expected timeout")?
                    .status,
                504
            );
        }
        assert_eq!(snapshot(&db)?, before);
        assert_eq!(
            storage
                .admissions
                .lock()
                .map_err(|_| Error::new(500, "test mutex poisoned"))?[1]
                .1,
            Duration::from_millis(400)
        );
        Ok(())
    }
    #[test]
    fn password_change_and_activity_touch_roll_back_after_cumulative_timeout() -> TestResult {
        let (_dir, app, db) = fixture()?;
        app.bootstrap_admin("admin", "test-password-123")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"admin", "password":"test-password-123"}),
        )?;
        let token = login["session"].as_str().ok_or("session")?;
        let identity = app.authenticate_session(token)?;
        let before = snapshot(&db)?;
        let hash: String =
            db.query_row("SELECT password_hash FROM gl_passwords", [], |r| r.get(0))?;
        let (limited, _) = timed(&app, 600);
        {
            let _clock = Clock::start();
            assert_eq!(limited.auth_operation(Some(&identity), "change_password", json!({"current_password":"test-password-123", "new_password":"test-password-456"})).err().ok_or("expected timeout")?.status, 504);
        }
        assert_eq!(snapshot(&db)?, before);
        assert_eq!(
            db.query_row("SELECT password_hash FROM gl_passwords", [], |r| r
                .get::<_, String>(0))?,
            hash
        );
        let old = ago(2);
        db.execute("UPDATE gl_sessions SET last_seen_at=?1", [&old])?;
        {
            let _clock = Clock::start();
            assert_eq!(
                limited
                    .authenticate_session(token)
                    .err()
                    .ok_or("expected timeout")?
                    .status,
                504
            );
        }
        assert_eq!(
            db.query_row("SELECT last_seen_at FROM gl_sessions", [], |r| r
                .get::<_, String>(0))?,
            old
        );
        Ok(())
    }
    #[test]
    fn read_only_auth_uses_one_transaction_and_commit_checks_deadline() -> TestResult {
        let (_dir, app, db) = fixture()?;
        app.bootstrap_admin("admin", "test-password-123")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"admin", "password":"test-password-123"}),
        )?;
        let identity = app.authenticate_session(login["session"].as_str().ok_or("session")?)?;
        let (limited, storage) = timed(&app, 600);
        {
            let _clock = Clock::start();
            limited.auth_operation(Some(&identity), "me", json!({}))?;
        }
        assert_eq!(
            storage
                .admissions
                .lock()
                .map_err(|_| Error::new(500, "test mutex poisoned"))?
                .len(),
            1
        );
        let before = snapshot(&db)?;
        let limited = app
            .clone()
            .with_timeout(Duration::from_secs(1))
            .with_response_reservation(Arc::new(|_| {
                Clock::advance(Duration::from_secs(2));
                Ok(())
            }));
        {
            let _clock = Clock::start();
            assert_eq!(
                limited
                    .auth_operation(
                        None,
                        "login_password",
                        json!({"username":"admin", "password":"test-password-123"})
                    )
                    .err()
                    .ok_or("expected timeout")?
                    .status,
                504
            );
        }
        assert_eq!(snapshot(&db)?, before);
        Ok(())
    }
    #[test]
    fn session_touch_rechecks_threshold_for_stale_readers() -> TestResult {
        let (_dir, app, db) = fixture()?;
        app.bootstrap_admin("admin", "test-password-123")?;
        let login = app.auth_operation(
            None,
            "login_password",
            json!({"username":"admin", "password":"test-password-123"}),
        )?;
        let identity = app.authenticate_session(login["session"].as_str().ok_or("session")?)?;
        let id = identity.session_id.ok_or("session")?;
        db.execute("UPDATE gl_sessions SET last_seen_at=?1", [ago(2)])?;
        db.execute_batch("CREATE TABLE touch_count(n INTEGER); INSERT INTO touch_count VALUES(0); CREATE TRIGGER count_touch AFTER UPDATE OF last_seen_at ON gl_sessions BEGIN UPDATE touch_count SET n=n+1; END;")?;
        let time = Utc::now();
        for milliseconds in 0..10 {
            let time = time + chrono::Duration::milliseconds(milliseconds);
            let now = time.to_rfc3339_opts(SecondsFormat::Millis, true);
            let before =
                (time - chrono::Duration::minutes(1)).to_rfc3339_opts(SecondsFormat::Millis, true);
            let mut t = app.storage.begin(false, Duration::from_secs(5))?;
            t.auth_touch_session(&id, &now, &before)?;
            t.commit()?;
        }
        assert_eq!(
            db.query_row("SELECT n FROM touch_count", [], |r| r.get::<_, i64>(0))?,
            1
        );
        Ok(())
    }
}
