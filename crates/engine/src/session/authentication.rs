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

impl CredentialRepository for SqlTransaction {
    fn lock_credentials(&mut self) -> Result<()> {
        if matches!(self.0.backend, Backend::Postgis(_)) {
            self.query_one("SELECT pg_advisory_xact_lock(7885)::text", &[])?;
        }
        Ok(())
    }

    fn auth_user(&mut self, id: &str) -> Result<Option<crate::authentication::UserRecord>> {
        self.query_opt(&format!("{} WHERE u.id=$1", USER_SELECT), &[&id])?
            .map(user_row)
            .transpose()
    }

    fn auth_username(
        &mut self,
        username: &str,
    ) -> Result<Option<crate::authentication::UserRecord>> {
        self.query_opt(
            &format!("{} WHERE u.username=$1", USER_SELECT),
            &[&username],
        )?
        .map(user_row)
        .transpose()
    }

    fn auth_users(&mut self, after: &str, limit: i64, status: &str) -> Result<Vec<crate::User>> {
        let select = "SELECT id,username,display_name,status,is_admin,must_change_password,created_at FROM gl_users";
        let rows = if status.is_empty() {
            self.query(
                &format!("{select} WHERE id>$1 ORDER BY id LIMIT $2"),
                &[&after, &limit],
            )?
        } else {
            self.query(
                &format!("{select} WHERE status=$3 AND id>$1 ORDER BY id LIMIT $2"),
                &[&after, &limit, &status],
            )?
        };
        rows.into_iter().map(|r| public_user_row(&r)).collect()
    }

    fn auth_insert_user(&mut self, r: &crate::authentication::UserRecord) -> Result<()> {
        let u = &r.user;
        self.execute("INSERT INTO gl_users(id,username,display_name,status,is_admin,must_change_password,created_at,credential_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&u.id,&u.username,&u.display_name,&u.status,&u.is_admin,&u.must_change_password,&u.created_at,&r.credential_version])?;
        self.execute(
            "INSERT INTO gl_passwords(user_id,password_hash) VALUES($1,$2)",
            &[&u.id, &r.password_hash],
        )
    }

    fn auth_update_user(&mut self, r: &crate::authentication::UserRecord) -> Result<()> {
        let u = &r.user;
        self.execute("UPDATE gl_users SET username=$2,display_name=$3,status=$4,is_admin=$5,must_change_password=$6,credential_version=$7 WHERE id=$1", &[&u.id,&u.username,&u.display_name,&u.status,&u.is_admin,&u.must_change_password,&r.credential_version])?;
        self.execute(
            "UPDATE gl_passwords SET password_hash=$2 WHERE user_id=$1",
            &[&u.id, &r.password_hash],
        )
    }

    fn auth_admin_count(&mut self) -> Result<i64> {
        self.query_one(
            "SELECT count(*) FROM gl_users WHERE status='active' AND is_admin",
            &[],
        )?
        .get(0usize)
    }

    fn auth_membership_target(&mut self, id: &str) -> Result<bool> {
        let suffix = if matches!(self.0.backend, Backend::Postgis(_)) {
            " FOR SHARE"
        } else {
            ""
        };
        Ok(self
            .query_opt(
                &format!("SELECT id FROM gl_users WHERE id=$1 AND status='active'{suffix}"),
                &[&id],
            )?
            .is_some())
    }

    fn auth_user_count(&mut self) -> Result<i64> {
        self.query_one("SELECT count(*) FROM gl_users", &[])?
            .get(0usize)
    }

    fn auth_key(&mut self, id: &str) -> Result<Option<crate::ApiKey>> {
        self.query_opt(&format!("{} WHERE id=$1", KEY_SELECT), &[&id])?
            .map(key_row)
            .transpose()
    }

    fn auth_key_digest(&mut self, digest: &str, now: &str) -> Result<Option<crate::ApiKey>> {
        self.query_opt(&format!("{} WHERE digest=$1 AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>$2)",KEY_SELECT), &[&digest,&now])?.map(key_row).transpose()
    }

    fn auth_keys(&mut self, user: &str, after: &str, limit: i64) -> Result<Vec<crate::ApiKey>> {
        self.query(
            &format!(
                "{} WHERE user_id=$1 AND id>$2 ORDER BY id LIMIT $3",
                KEY_SELECT
            ),
            &[&user, &after, &limit],
        )?
        .into_iter()
        .map(key_row)
        .collect()
    }

    fn auth_insert_key(&mut self, k: &crate::ApiKey, digest: &str) -> Result<()> {
        self.execute("INSERT INTO gl_api_keys(id,user_id,digest,name,key_prefix,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)", &[&k.id,&k.user_id,&digest,&k.name,&k.key_prefix,&k.created_at,&k.expires_at])
    }

    fn auth_update_key(&mut self, k: &crate::ApiKey) -> Result<()> {
        self.execute(
            "UPDATE gl_api_keys SET name=$2,expires_at=$3,revoked_at=$4 WHERE id=$1",
            &[&k.id, &k.name, &k.expires_at, &k.revoked_at],
        )
    }

    fn auth_delete_key(&mut self, id: &str) -> Result<()> {
        self.execute("DELETE FROM gl_api_keys WHERE id=$1", &[&id])
    }

    fn auth_touch_key(&mut self, id: &str, now: &str, before: &str) -> Result<()> {
        self.execute("UPDATE gl_api_keys SET last_used_at=$2 WHERE id=$1 AND (last_used_at IS NULL OR last_used_at<$3)", &[&id,&now,&before])
    }

    fn auth_session(
        &mut self,
        digest: &str,
    ) -> Result<Option<crate::authentication::SessionRecord>> {
        self.query_opt("SELECT id,user_id,source_key_id,credential_version,created_at,last_seen_at,expires_at FROM gl_sessions WHERE digest=$1", &[&digest])?.map(|r| Ok(crate::authentication::SessionRecord {id:r.get(0usize)?,user_id:r.get(1usize)?,source_key_id:r.get(2usize)?,credential_version:r.get(3usize)?,created_at:r.get(4usize)?,last_seen_at:r.get(5usize)?,expires_at:r.get(6usize)?})).transpose()
    }

    fn auth_insert_session(
        &mut self,
        s: &crate::authentication::SessionRecord,
        digest: &str,
    ) -> Result<()> {
        // Bound cleanup work per login; the expiry index avoids scanning live sessions.
        self.execute(
            "DELETE FROM gl_sessions WHERE id IN (SELECT id FROM gl_sessions WHERE expires_at<=$1 ORDER BY expires_at LIMIT 100)",
            &[&s.created_at],
        )?;
        self.execute("INSERT INTO gl_sessions(id,user_id,digest,source_key_id,credential_version,created_at,last_seen_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)", &[&s.id,&s.user_id,&digest,&s.source_key_id,&s.credential_version,&s.created_at,&s.last_seen_at,&s.expires_at])
    }

    fn auth_touch_session(&mut self, id: &str, now: &str, before: &str) -> Result<()> {
        self.execute(
            "UPDATE gl_sessions SET last_seen_at=$2 WHERE id=$1 AND last_seen_at<$3",
            &[&id, &now, &before],
        )
    }

    fn auth_delete_session(&mut self, id: &str) -> Result<()> {
        self.execute("DELETE FROM gl_sessions WHERE id=$1", &[&id])
    }

    fn auth_invalidate_user(&mut self, id: &str, now: &str, keys: bool) -> Result<()> {
        if keys {
            self.execute(
                "UPDATE gl_api_keys SET revoked_at=$2 WHERE user_id=$1 AND revoked_at IS NULL",
                &[&id, &now],
            )?;
        }
        self.execute("DELETE FROM gl_sessions WHERE user_id=$1", &[&id])
    }

    fn auth_audit(&mut self, actor: &str, action: &str, resource: &str, now: &str) -> Result<()> {
        self.execute(
            "INSERT INTO gl_auth_audit(id,actor,action,resource,created_at) VALUES($1,$2,$3,$4,$5)",
            &[
                &uuid::Uuid::new_v4().to_string(),
                &actor,
                &action,
                &resource,
                &now,
            ],
        )
    }
}
