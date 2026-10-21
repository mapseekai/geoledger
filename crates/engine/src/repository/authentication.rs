use crate::Result;

/// Global users, API keys, sessions and their audit; the credential lock lasts until transaction end.
pub trait CredentialRepository: Send {
    /// Serialize global credential mutations and their audit/receipt writes until commit.
    fn lock_credentials(&mut self) -> Result<()>;

    fn auth_user(&mut self, id: &str) -> Result<Option<crate::authentication::UserRecord>>;

    fn auth_username(
        &mut self,
        username: &str,
    ) -> Result<Option<crate::authentication::UserRecord>>;

    fn auth_users(&mut self, after: &str, limit: i64, status: &str) -> Result<Vec<crate::User>>;

    fn auth_insert_user(&mut self, user: &crate::authentication::UserRecord) -> Result<()>;

    fn auth_update_user(&mut self, user: &crate::authentication::UserRecord) -> Result<()>;

    fn auth_admin_count(&mut self) -> Result<i64>;

    fn auth_membership_target(&mut self, id: &str) -> Result<bool>;

    fn auth_user_count(&mut self) -> Result<i64>;

    fn auth_key(&mut self, id: &str) -> Result<Option<crate::ApiKey>>;

    fn auth_key_digest(&mut self, digest: &str, now: &str) -> Result<Option<crate::ApiKey>>;

    fn auth_keys(&mut self, user: &str, after: &str, limit: i64) -> Result<Vec<crate::ApiKey>>;

    fn auth_insert_key(&mut self, key: &crate::ApiKey, digest: &str) -> Result<()>;

    fn auth_update_key(&mut self, key: &crate::ApiKey) -> Result<()>;

    fn auth_delete_key(&mut self, id: &str) -> Result<()>;

    fn auth_touch_key(&mut self, id: &str, now: &str, before: &str) -> Result<()>;

    fn auth_session(
        &mut self,
        digest: &str,
    ) -> Result<Option<crate::authentication::SessionRecord>>;

    fn auth_insert_session(
        &mut self,
        session: &crate::authentication::SessionRecord,
        digest: &str,
    ) -> Result<()>;

    fn auth_touch_session(&mut self, id: &str, now: &str, before: &str) -> Result<()>;

    fn auth_delete_session(&mut self, id: &str) -> Result<()>;

    fn auth_invalidate_user(&mut self, id: &str, now: &str, keys: bool) -> Result<()>;

    fn auth_audit(&mut self, actor: &str, action: &str, resource: &str, now: &str) -> Result<()>;
}
