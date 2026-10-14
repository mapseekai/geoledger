//! SQL identity persistence shared by SQLite and PostGIS.
use super::*;
use crate::authentication::UserRecord;
use crate::{ApiKey, User};

pub(super) fn user_row(r: Row) -> Result<UserRecord> {
    Ok(UserRecord {
        user: public_user_row(&r)?,
        credential_version: r.get(7usize)?,
        password_hash: r.get(8usize)?,
    })
}
pub(super) fn key_row(r: Row) -> Result<ApiKey> {
    Ok(ApiKey {
        id: r.get(0usize)?,
        user_id: r.get(1usize)?,
        name: r.get(2usize)?,
        key_prefix: r.get(3usize)?,
        created_at: r.get(4usize)?,
        expires_at: r.get(5usize)?,
        revoked_at: r.get(6usize)?,
        last_used_at: r.get(7usize)?,
    })
}
pub(super) const USER_SELECT: &str = "SELECT u.id,u.username,u.display_name,u.status,u.is_admin,u.must_change_password,u.created_at,u.credential_version,p.password_hash FROM gl_users u JOIN gl_passwords p ON p.user_id=u.id";
pub(super) const KEY_SELECT: &str = "SELECT id,user_id,name,key_prefix,created_at,expires_at,revoked_at,last_used_at FROM gl_api_keys";

pub(super) fn public_user_row(r: &Row) -> Result<User> {
    Ok(User {
        id: r.get(0usize)?,
        username: r.get(1usize)?,
        display_name: r.get(2usize)?,
        status: r.get(3usize)?,
        is_admin: r.get(4usize)?,
        must_change_password: r.get(5usize)?,
        created_at: r.get(6usize)?,
    })
}
