//! Explicit, opt-in tenant membership and role policies for the native CRUD.
//! A global user role never grants access to another tenant.
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Membership {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub role: String,
}

fn valid_slug(value: &str) -> bool {
    (2..=64).contains(&value.len())
        && value.starts_with(|ch: char| ch.is_ascii_lowercase())
        && value.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn valid_role(role: &str) -> bool {
    matches!(role, "reader" | "editor" | "admin")
}

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS rux_tenants (
            id TEXT PRIMARY KEY,
            slug TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS rux_tenant_memberships (
            tenant_id TEXT NOT NULL REFERENCES rux_tenants(id),
            user_id TEXT NOT NULL REFERENCES rux_users(id),
            role TEXT NOT NULL CHECK(role IN ('reader','editor','admin')),
            revoked_at INTEGER,
            PRIMARY KEY(tenant_id,user_id)
        );
        CREATE INDEX IF NOT EXISTS rux_tenant_memberships_user_idx ON rux_tenant_memberships(user_id);"
    )
}

pub fn create(db: &Connection, slug: &str, name: &str) -> Result<String, String> {
    if !valid_slug(slug) {
        return Err("Tenant slug must be 2-64 lowercase letters, digits or '-' and start with a letter".into());
    }
    if name.trim().is_empty() || name.len() > 120 {
        return Err("Tenant name must contain 1-120 UTF-8 bytes".into());
    }
    initialize(db).map_err(|error| error.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO rux_tenants (id,slug,name,created_at) VALUES (?1,?2,?3,?4)",
        params![id,slug,name.trim(),super::user_auth::unix_now()],
    ).map_err(|error| format!("Cannot create tenant (slug may already exist): {error}"))?;
    Ok(id)
}

pub fn grant(db: &Connection, slug: &str, username: &str, role: &str) -> Result<(), String> {
    if !valid_role(role) { return Err("Tenant role must be reader, editor, or admin".into()); }
    initialize(db).map_err(|e| e.to_string())?;
    let tenant_id: String = db.query_row("SELECT id FROM rux_tenants WHERE slug = ?1 AND active = 1",
        params![slug], |row| row.get(0)).optional()
        .map_err(|e| e.to_string())?
        .ok_or("Active tenant not found")?;
    let user_id: String = db.query_row("SELECT id FROM rux_users WHERE username = ?1 AND disabled = 0",
        params![username], |row| row.get(0)).optional()
        .map_err(|e| e.to_string())?
        .ok_or("Active user not found")?;
    db.execute(
        "INSERT INTO rux_tenant_memberships (tenant_id,user_id,role,revoked_at)
         VALUES (?1,?2,?3,NULL)
         ON CONFLICT(tenant_id,user_id) DO UPDATE SET role=excluded.role, revoked_at=NULL",
        params![tenant_id,user_id,role],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn revoke(db: &Connection, slug: &str, username: &str) -> Result<bool, String> {
    initialize(db).map_err(|e| e.to_string())?;
    let changed = db.execute(
        "UPDATE rux_tenant_memberships SET revoked_at = ?1
         WHERE tenant_id = (SELECT id FROM rux_tenants WHERE slug = ?2)
           AND user_id = (SELECT id FROM rux_users WHERE username = ?3)
           AND revoked_at IS NULL",
        params![super::user_auth::unix_now(),slug,username],
    ).map_err(|e| e.to_string())?;
    Ok(changed > 0)
}

pub fn memberships(db: &Connection, user_id: &str) -> rusqlite::Result<Vec<Membership>> {
    let mut stmt = db.prepare(
        "SELECT t.id,t.slug,t.name,m.role
         FROM rux_tenant_memberships m
         JOIN rux_tenants t ON t.id = m.tenant_id
         JOIN rux_users u ON u.id = m.user_id
         WHERE m.user_id = ?1 AND m.revoked_at IS NULL AND t.active = 1 AND u.disabled = 0
         ORDER BY t.name,t.id"
    )?;
    let rows = stmt.query_map(params![user_id], |row| Ok(Membership {
        id: row.get(0)?, slug: row.get(1)?, name: row.get(2)?, role: row.get(3)?,
    }))?;
    rows.collect()
}

pub fn membership(db: &Connection, user_id: &str, tenant_id: &str) -> rusqlite::Result<Option<Membership>> {
    db.query_row(
        "SELECT t.id,t.slug,t.name,m.role
         FROM rux_tenant_memberships m
         JOIN rux_tenants t ON t.id = m.tenant_id
         JOIN rux_users u ON u.id = m.user_id
         WHERE m.user_id = ?1 AND t.id = ?2
           AND m.revoked_at IS NULL AND t.active = 1 AND u.disabled = 0",
        params![user_id,tenant_id],
        |row| Ok(Membership { id:row.get(0)?, slug:row.get(1)?, name:row.get(2)?, role:row.get(3)? })
    ).optional()
}

pub fn active_count(db: &Connection) -> rusqlite::Result<i64> {
    initialize(db)?;
    db.query_row(
        "SELECT COUNT(*) FROM rux_tenant_memberships m
         JOIN rux_users u ON u.id = m.user_id
         JOIN rux_tenants t ON t.id = m.tenant_id
         WHERE m.revoked_at IS NULL AND t.active = 1 AND u.disabled = 0", [],
        |row| row.get(0),
    )
}

pub fn list(db: &Connection) -> Result<Vec<(String,String,bool)>, String> {
    initialize(db).map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT slug,name,active FROM rux_tenants ORDER BY slug")
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?,row.get(1)?,row.get::<_,i64>(2)? != 0)))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

fn project_db(project: &Path) -> Result<Connection, String> {
    let file = project.join("records.sqlite");
    if !file.is_file() { return Err(format!("Database not found: {}. Start the app once.", file.display())); }
    let db = Connection::open(file).map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON;").map_err(|e| e.to_string())?;
    Ok(db)
}

pub fn create_in_project(project: &Path, slug: &str, name: &str) -> Result<String,String> {
    create(&project_db(project)?,slug,name)
}
pub fn grant_in_project(project: &Path, slug: &str, username: &str, role: &str) -> Result<(),String> {
    grant(&project_db(project)?,slug,username,role)
}
pub fn revoke_in_project(project: &Path, slug: &str, username: &str) -> Result<bool,String> {
    revoke(&project_db(project)?,slug,username)
}
pub fn list_in_project(project: &Path) -> Result<Vec<(String,String,bool)>,String> {
    list(&project_db(project)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn membership_is_tenant_scoped_even_for_global_admins() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        crate::user_auth::create_user(&db,"alice","test-user-password-123","admin").unwrap();
        crate::user_auth::create_user(&db,"bob","test-user-password-123","reader").unwrap();
        let a = create(&db,"acme","Acme SA").unwrap();
        let b = create(&db,"other","Other Ltd").unwrap();
        grant(&db,"acme","alice","editor").unwrap();
        grant(&db,"other","bob","admin").unwrap();
        let alice_id: String = db.query_row("SELECT id FROM rux_users WHERE username='alice'", [], |row| row.get(0)).unwrap();
        let bob_id: String = db.query_row("SELECT id FROM rux_users WHERE username='bob'", [], |row| row.get(0)).unwrap();
        assert_eq!(memberships(&db,&alice_id).unwrap().len(),1);
        assert_eq!(membership(&db,&alice_id,&a).unwrap().unwrap().role,"editor");
        assert!(membership(&db,&alice_id,&b).unwrap().is_none());
        assert!(membership(&db,&bob_id,&a).unwrap().is_none());
        assert_eq!(membership(&db,&bob_id,&b).unwrap().unwrap().role,"admin");
        assert_eq!(active_count(&db).unwrap(),2);
        assert!(revoke(&db,"acme","alice").unwrap());
        assert!(membership(&db,&alice_id,&a).unwrap().is_none());
        assert!(!revoke(&db,"acme","alice").unwrap());
        assert!(grant(&db,"other","nobody","reader").is_err());
        assert!(grant(&db,"acme","bob","superadmin").is_err());
    }
}
