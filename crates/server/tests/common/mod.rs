#![allow(dead_code)]
use sha2::{Digest, Sha256};
use std::path::Path;
pub const KEY: &str = "glk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub fn tempdir() -> std::io::Result<tempfile::TempDir> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-fixtures/server");
    std::fs::create_dir_all(&root)?;
    tempfile::tempdir_in(root)
}
pub fn user_id(name: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Uuid::from_bytes(bytes).to_string()
}
// Transport regression fixtures use fixed public test keys; lifecycle tests use the real auth API.
pub fn seed(
    path: &Path,
    users: &[(&str, &str, bool)],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = rusqlite::Connection::open(path)?;
    for (name, key, admin) in users {
        let id = user_id(name);
        connection.execute("INSERT OR IGNORE INTO gl_users(id,username,display_name,status,is_admin,must_change_password,created_at) VALUES (?1,?2,?2,'active',?3,false,'2026-01-01T00:00:00Z')",rusqlite::params![id,name,admin])?;
        connection.execute("INSERT OR IGNORE INTO gl_passwords(user_id,password_hash) VALUES (?1,'unused-password-fixture')",[&id])?;
        connection.execute("INSERT OR IGNORE INTO gl_api_keys(id,user_id,digest,name,key_prefix,created_at,last_used_at) VALUES (?1,?2,?3,'test',?4,'2026-01-01T00:00:00Z','2099-01-01T00:00:00Z')",rusqlite::params![uuid::Uuid::new_v4().to_string(),id,Sha256::digest(key.as_bytes()).iter().map(|b|format!("{b:02x}")).collect::<String>(),&key[..12]])?;
    }
    Ok(())
}
pub fn database(path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    geoledger_engine::Application::new(geoledger_engine::Storage::Sqlite(path.to_owned()))
        .migrate()?;
    seed(path, &[("ops", KEY, true)])
}
