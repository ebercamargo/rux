//! Browser-user authentication for the experimental RUX native CRUD.
//! Local administrator provisioning only; no HTTP self-registration.
use argon2::{password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString}, Argon2};
use rand_core::OsRng;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{path::Path, time::{SystemTime, UNIX_EPOCH}};

pub const SESSION_SECONDS: i64 = 8 * 60 * 60;
const MAX_FAILURES: i64 = 5;
const LOCK_SECONDS: i64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub username: String,
    pub role: String,
    pub expires_at: i64,
    pub csrf: String,
}

#[derive(Debug, Clone)]
pub struct Login {
    pub token: String,
    pub session: Session,
}

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn random_token() -> String {
    format!("ruxs_{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn csrf_for(token: &str) -> String {
    digest(&format!("rux-csrf-v1:{token}"))
}

fn valid_username(username: &str) -> bool {
    (3..=64).contains(&username.len())
        && username.starts_with(|c: char| c.is_ascii_alphabetic())
        && username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_role(role: &str) -> bool {
    matches!(role, "reader" | "editor" | "admin")
}

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS rux_users (
             id TEXT PRIMARY KEY,
             username TEXT NOT NULL UNIQUE,
             password_hash TEXT NOT NULL,
             role TEXT NOT NULL CHECK(role IN ('reader','editor','admin')),
             disabled INTEGER NOT NULL DEFAULT 0,
             failures INTEGER NOT NULL DEFAULT 0,
             locked_until INTEGER NOT NULL DEFAULT 0,
             created_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS rux_sessions (
             token_hash TEXT PRIMARY KEY,
             user_id TEXT NOT NULL REFERENCES rux_users(id),
             issued_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             revoked_at INTEGER
         );
         CREATE INDEX IF NOT EXISTS rux_sessions_user_idx ON rux_sessions(user_id);"
    )
}

pub fn create_user(db: &Connection, username: &str, password: &str, role: &str) -> Result<(), String> {
    if !valid_username(username) {
        return Err("Username must be 3-64 ASCII letters, digits, '-' or '_' and start with a letter".into());
    }
    if !valid_role(role) {
        return Err("Role must be reader, editor, or admin".into());
    }
    if !(12..=128).contains(&password.len()) {
        return Err("Password must be between 12 and 128 UTF-8 bytes".into());
    }
    initialize(db).map_err(|e| e.to_string())?;
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| "Failed to hash password".to_string())?
        .to_string();
    db.execute(
        "INSERT INTO rux_users (id,username,password_hash,role,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![uuid::Uuid::new_v4().to_string(), username, password_hash, role, unix_now()],
    ).map_err(|e| format!("Cannot create user (username may already exist): {e}"))?;
    Ok(())
}

pub fn enabled_count(db: &Connection) -> rusqlite::Result<i64> {
    initialize(db)?;
    db.query_row("SELECT COUNT(*) FROM rux_users WHERE disabled = 0", [], |row| row.get(0))
}

pub fn login(db: &Connection, username: &str, password: &str, now: i64) -> Result<Option<Login>, String> {
    initialize(db).map_err(|e| e.to_string())?;
    if !valid_username(username) || password.len() > 128 { return Ok(None); }
    let info: Option<(String, String, String, i64, i64, i64)> = db.query_row(
        "SELECT id,password_hash,role,disabled,failures,locked_until FROM rux_users WHERE username = ?1",
        params![username], |row| {
            Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))
        }
    ).optional().map_err(|e| e.to_string())?;
    let Some((user_id,password_hash,role,disabled,failures,locked_until)) = info else { return Ok(None); };
    if disabled != 0 || locked_until > now { return Ok(None); }
    let parsed = PasswordHash::new(&password_hash)
        .map_err(|_| "Stored password hash is invalid".to_string())?;
    let valid = Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok();
    if !valid {
        let next = failures + 1;
        let (failures, locked_until) = if next >= MAX_FAILURES { (0, now + LOCK_SECONDS) } else { (next, 0) };
        db.execute(
            "UPDATE rux_users SET failures = ?1, locked_until = ?2 WHERE id = ?3",
            params![failures, locked_until, user_id],
        ).map_err(|e| e.to_string())?;
        return Ok(None);
    }
    db.execute("UPDATE rux_users SET failures = 0, locked_until = 0 WHERE id = ?1", params![user_id])
        .map_err(|e| e.to_string())?;
    db.execute("DELETE FROM rux_sessions WHERE expires_at <= ?1", params![now])
        .map_err(|e| e.to_string())?;
    let token = random_token();
    let expires_at = now + SESSION_SECONDS;
    db.execute(
        "INSERT INTO rux_sessions (token_hash,user_id,issued_at,expires_at) VALUES (?1,?2,?3,?4)",
        params![digest(&token),user_id,now,expires_at],
    ).map_err(|e| e.to_string())?;
    Ok(Some(Login {
        session: Session { username: username.into(), role, expires_at, csrf: csrf_for(&token) },
        token,
    }))
}

pub fn session(db: &Connection, token: &str, now: i64) -> Result<Option<Session>, String> {
    if !token.starts_with("ruxs_") || token.len() != 69 || !token[5..].bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let user: Option<(String,String,i64)> = db.query_row(
        "SELECT u.username,u.role,s.expires_at FROM rux_sessions s JOIN rux_users u ON u.id = s.user_id
         WHERE s.token_hash = ?1 AND s.revoked_at IS NULL AND s.expires_at > ?2 AND u.disabled = 0",
        params![digest(token),now], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional().map_err(|e| e.to_string())?;
    Ok(user.map(|(username,role,expires_at)| Session {
        username, role, expires_at, csrf: csrf_for(token),
    }))
}

pub fn csrf_matches(expected: &str, provided: &str) -> bool {
    if expected.len() != provided.len() { return false; }
    let mut result = 0u8;
    for (&a, &b) in expected.as_bytes().iter().zip(provided.as_bytes()) { result |= a ^ b; }
    result == 0
}

pub fn can(role: &str, operation: &str) -> bool {
    match operation {
        "GET" | "HEAD" => valid_role(role),
        "POST" | "PUT" | "PATCH" => matches!(role, "editor" | "admin"),
        "DELETE" | "METRICS" => role == "admin",
        _ => false,
    }
}

pub fn logout(db: &Connection, token: &str) -> Result<(), String> {
    db.execute("UPDATE rux_sessions SET revoked_at = ?1 WHERE token_hash = ?2 AND revoked_at IS NULL",
        params![unix_now(),digest(token)]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn disable(db: &Connection, username: &str) -> Result<bool, String> {
    let count = db.execute("UPDATE rux_users SET disabled = 1 WHERE username = ?1 AND disabled = 0", params![username])
        .map_err(|e| e.to_string())?;
    if count > 0 {
        db.execute(
            "UPDATE rux_sessions SET revoked_at = ?1 WHERE user_id = (SELECT id FROM rux_users WHERE username = ?2) AND revoked_at IS NULL",
            params![unix_now(),username],
        ).map_err(|e| e.to_string())?;
    }
    Ok(count > 0)
}

pub fn list(db: &Connection) -> Result<Vec<(String,String,bool)>, String> {
    initialize(db).map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT username,role,disabled FROM rux_users ORDER BY username").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get::<_,i64>(2)? != 0)))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

fn project_db(project: &Path) -> Result<Connection, String> {
    let file = project.join("records.sqlite");
    if !file.is_file() { return Err(format!("Database not found: {}. Start the application once first.", file.display())); }
    Connection::open(file).map_err(|e| e.to_string())
}

pub fn create_in_project(project: &Path, name: &str, password: &str, role: &str) -> Result<(), String> {
    create_user(&project_db(project)?,name,password,role)
}
pub fn disable_in_project(project: &Path, name: &str) -> Result<bool, String> {
    disable(&project_db(project)?,name)
}
pub fn list_in_project(project: &Path) -> Result<Vec<(String,String,bool)>, String> {
    list(&project_db(project)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_sessions_expire_and_logout_revokes() {
        let db = Connection::open_in_memory().unwrap();
        create_user(&db, "alice", "a reasonably long secret!", "reader").unwrap();
        assert!(login(&db,"alice","incorrect password",1000).unwrap().is_none());
        let auth = login(&db,"alice","a reasonably long secret!",1000).unwrap().unwrap();
        assert!(auth.token.starts_with("ruxs_"));
        assert_eq!(auth.token.len(),69);
        let active = session(&db,&auth.token,1000).unwrap().unwrap();
        assert_eq!(active.username,"alice");
        assert_eq!(active.role,"reader");
        assert!(can(&active.role,"GET"));
        assert!(!can(&active.role,"POST"));
        assert!(!can(&active.role,"DELETE"));
        assert!(csrf_matches(&active.csrf,&auth.session.csrf));
        assert!(!csrf_matches(&active.csrf,"incorrect"));
        assert!(session(&db,&auth.token,1000 + SESSION_SECONDS).unwrap().is_none());
        logout(&db,&auth.token).unwrap();
        assert!(session(&db,&auth.token,1000).unwrap().is_none());
        let stored: String = db.query_row("SELECT token_hash FROM rux_sessions", [], |row| row.get(0)).unwrap();
        assert_ne!(stored,auth.token);
    }

    #[test]
    fn failed_logins_lock_account_temporarily() {
        let db = Connection::open_in_memory().unwrap();
        create_user(&db,"bob","long-enough-passphrase","admin").unwrap();
        for _ in 0..5 { assert!(login(&db,"bob","wrong",500).unwrap().is_none()); }
        assert!(login(&db,"bob","long-enough-passphrase",501).unwrap().is_none());
        assert!(login(&db,"bob","long-enough-passphrase",801).unwrap().is_some());
    }

    #[test]
    fn disabling_user_revokes_all_sessions() {
        let db = Connection::open_in_memory().unwrap();
        create_user(&db,"diana","even-longer-password","editor").unwrap();
        let first=login(&db,"diana","even-longer-password",100).unwrap().unwrap();
        assert_eq!(enabled_count(&db).unwrap(),1);
        assert!(disable(&db,"diana").unwrap());
        assert!(session(&db,&first.token,101).unwrap().is_none());
        assert!(login(&db,"diana","even-longer-password",101).unwrap().is_none());
        assert_eq!(enabled_count(&db).unwrap(),0);
        assert!(!disable(&db,"diana").unwrap());
        assert!(!can("editor","METRICS"));
        assert!(can("admin","METRICS"));
    }
}
