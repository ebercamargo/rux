//! Per-principal API credentials for the native CRUD runtime.
//! This is not a password/login/session system. Tokens are high-entropy API credentials.
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Unauthenticated,
    Forbidden,
}

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS rux_identities (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            role TEXT NOT NULL CHECK(role IN ('reader', 'editor', 'admin')),
            token_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            revoked_at TEXT
        );",
    )
}

fn digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn valid_name(name: &str) -> bool {
    (3..=64).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_role(role: &str) -> bool {
    matches!(role, "reader" | "editor" | "admin")
}

pub fn create(db: &Connection, name: &str, role: &str) -> Result<String, String> {
    if !valid_name(name) { return Err("Identity name must be 3-64 ASCII letters, numbers, '_' or '-' and start with a letter".into()); }
    if !valid_role(role) { return Err("Role must be reader, editor, or admin".into()); }
    initialize(db).map_err(|e| e.to_string())?;
    // Two independent UUIDv4 values provide high-entropy API credentials.
    let token = format!("rux_{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    db.execute(
        "INSERT INTO rux_identities (id, name, role, token_hash) VALUES (?1, ?2, ?3, ?4)",
        params![uuid::Uuid::new_v4().to_string(), name, role, digest(&token)],
    ).map_err(|error| format!("Cannot create identity (name may already exist): {error}"))?;
    Ok(token)
}

pub fn active_count(db: &Connection) -> rusqlite::Result<i64> {
    initialize(db)?;
    db.query_row(
        "SELECT COUNT(*) FROM rux_identities WHERE revoked_at IS NULL", [],
        |row| row.get(0),
    )
}

pub fn revoke(db: &Connection, name: &str) -> Result<bool, String> {
    initialize(db).map_err(|e| e.to_string())?;
    db.execute(
        "UPDATE rux_identities SET revoked_at = datetime('now') WHERE name = ?1 AND revoked_at IS NULL",
        params![name],
    ).map(|changed| changed > 0).map_err(|e| e.to_string())
}

pub fn list(db: &Connection) -> Result<Vec<(String, String, bool)>, String> {
    initialize(db).map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT name, role, revoked_at IS NOT NULL FROM rux_identities ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

pub fn authorize(db: &Connection, header: Option<&str>, operation: &str) -> rusqlite::Result<Decision> {
    let Some(token) = header.and_then(|header| header.strip_prefix("Bearer ")) else {
        return Ok(Decision::Unauthenticated);
    };
    let Some(hex) = token.strip_prefix("rux_") else { return Ok(Decision::Unauthenticated); };
    if hex.len() != 64 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Ok(Decision::Unauthenticated);
    }
    let role: Option<String> = db.query_row(
        "SELECT role FROM rux_identities WHERE token_hash = ?1 AND revoked_at IS NULL",
        params![digest(token)],
        |row| row.get(0),
    ).optional()?;
    let Some(role) = role else { return Ok(Decision::Unauthenticated); };
    let permitted = match operation {
        "GET" | "HEAD" => matches!(role.as_str(), "reader" | "editor" | "admin"),
        "POST" | "PUT" | "PATCH" => matches!(role.as_str(), "editor" | "admin"),
        "DELETE" | "METRICS" => role == "admin",
        _ => false,
    };
    Ok(if permitted { Decision::Allowed } else { Decision::Forbidden })
}

fn open_project_db(project: &Path) -> Result<Connection, String> {
    let db_path = project.join("records.sqlite");
    if !db_path.is_file() {
        return Err(format!("Database not found: {}. Run the application once before managing identities.", db_path.display()));
    }
    Connection::open(db_path).map_err(|e| format!("Cannot open identity database: {e}"))
}

pub fn create_in_project(project: &Path, name: &str, role: &str) -> Result<String, String> {
    let db = open_project_db(project)?;
    create(&db, name, role)
}
pub fn revoke_in_project(project: &Path, name: &str) -> Result<bool, String> {
    let db = open_project_db(project)?;
    revoke(&db, name)
}
pub fn list_in_project(project: &Path) -> Result<Vec<(String, String, bool)>, String> {
    let db = open_project_db(project)?;
    list(&db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_scoped_and_revocation_is_immediate() {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        let viewer = create(&db, "viewer_one", "reader").unwrap();
        let editor = create(&db, "editor_one", "editor").unwrap();
        let admin = create(&db, "admin_one", "admin").unwrap();
        let header = |token: &str| format!("Bearer {token}");

        assert_eq!(authorize(&db, Some(&header(&viewer)), "GET").unwrap(), Decision::Allowed);
        assert_eq!(authorize(&db, Some(&header(&viewer)), "POST").unwrap(), Decision::Forbidden);
        assert_eq!(authorize(&db, Some(&header(&editor)), "PUT").unwrap(), Decision::Allowed);
        assert_eq!(authorize(&db, Some(&header(&editor)), "DELETE").unwrap(), Decision::Forbidden);
        assert_eq!(authorize(&db, Some(&header(&admin)), "DELETE").unwrap(), Decision::Allowed);
        assert_eq!(authorize(&db, Some(&header(&admin)), "METRICS").unwrap(), Decision::Allowed);
        assert_eq!(authorize(&db, Some(&header(&editor)), "METRICS").unwrap(), Decision::Forbidden);
        assert_eq!(authorize(&db, None, "GET").unwrap(), Decision::Unauthenticated);
        assert_eq!(authorize(&db, Some("Bearer fake"), "GET").unwrap(), Decision::Unauthenticated);
        assert!(!list(&db).unwrap().iter().any(|(name, _, _)| name.contains("rux_")));
        assert_eq!(active_count(&db).unwrap(), 3);
        assert!(revoke(&db, "editor_one").unwrap());
        assert_eq!(authorize(&db, Some(&header(&editor)), "GET").unwrap(), Decision::Unauthenticated);
        assert_eq!(active_count(&db).unwrap(), 2);
        assert!(!revoke(&db, "editor_one").unwrap());
    }

    #[test]
    fn stores_hashes_never_plaintext_tokens() {
        let db = Connection::open_in_memory().unwrap();
        let token = create(&db, "test_reader", "reader").unwrap();
        assert!(token.starts_with("rux_"));
        assert_eq!(token.len(), 68);
        let stored: String = db.query_row("SELECT token_hash FROM rux_identities", [], |row| row.get(0)).unwrap();
        assert_eq!(stored.len(), 64);
        assert_ne!(stored, token);
        assert_eq!(stored, digest(&token));
        assert!(create(&db, "test_reader", "reader").is_err());
        assert!(create(&db, "bad", "root").is_err());
        assert!(create(&db, "../unsafe", "reader").is_err());
    }
}
