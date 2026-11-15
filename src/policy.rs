//! Custom authorization roles and narrowly scoped permissions.
//! Role names have no implicit power: every allowed action is a persisted grant.
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub fn valid_role_name(role: &str) -> bool {
    (2..=64).contains(&role.len())
        && role.starts_with(|c: char| c.is_ascii_alphabetic())
        && role.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn valid_permission(permission: &str) -> bool {
    (3..=128).contains(&permission.len())
        && permission.split('.').count() >= 2
        && permission.split('.').all(|part| {
            !part.is_empty()
                && part.starts_with(|c: char| c.is_ascii_alphabetic())
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

pub fn operation_permission(method: &str) -> Option<&'static str> {
    match method {
        "GET" | "HEAD" => Some("records.read"),
        "POST" => Some("records.create"),
        "PUT" | "PATCH" => Some("records.update"),
        "DELETE" => Some("records.delete"),
        "METRICS" => Some("metrics.read"),
        _ => None,
    }
}

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS rux_roles (
            name TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS rux_role_permissions (
            role_name TEXT NOT NULL REFERENCES rux_roles(name) ON DELETE CASCADE,
            permission TEXT NOT NULL,
            PRIMARY KEY (role_name, permission)
        );
        CREATE TABLE IF NOT EXISTS rux_policy_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );"
    )?;
    let seeded: Option<String> = db.query_row(
        "SELECT value FROM rux_policy_meta WHERE key = 'default_roles_v1'", [],
        |row| row.get(0),
    ).optional()?;
    if seeded.is_none() {
        // Compatibility examples only. They can be edited or removed later.
        // They are never re-granted after a permission has been revoked.
        for role in ["reader", "editor", "admin"] {
            db.execute("INSERT OR IGNORE INTO rux_roles(name,created_at) VALUES (?1,?2)",
                params![role, super::user_auth::unix_now()])?;
        }
        for (role, permission) in [
            ("reader", "records.read"),
            ("editor", "records.read"),
            ("editor", "records.create"),
            ("editor", "records.update"),
            ("admin", "records.read"),
            ("admin", "records.create"),
            ("admin", "records.update"),
            ("admin", "records.delete"),
            ("admin", "records.access_all"),
            ("admin", "metrics.read"),
        ] {
            db.execute(
                "INSERT OR IGNORE INTO rux_role_permissions(role_name,permission) VALUES (?1,?2)",
                params![role, permission],
            )?;
        }
        db.execute(
            "INSERT OR IGNORE INTO rux_policy_meta(key,value) VALUES ('default_roles_v1','1')", [],
        )?;
    }
    Ok(())
}

/// Synchronizes source-managed roles exactly as declared by the RUX program.
/// Roles not declared in the current source lose all source-managed privileges,
/// but identities, memberships and the role names themselves are retained.
pub fn sync_source_roles(
    db: &mut Connection,
    spec: Option<&super::security_schema::SecuritySpec>,
) -> Result<(), String> {
    initialize(db).map_err(|error| error.to_string())?;
    let tx = db.transaction().map_err(|error| error.to_string())?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS rux_source_roles (
            role_name TEXT PRIMARY KEY REFERENCES rux_roles(name)
        );",
    ).map_err(|error| error.to_string())?;

    let prior_roles: Vec<String> = {
        let mut stmt = tx.prepare("SELECT role_name FROM rux_source_roles")
            .map_err(|error| error.to_string())?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?
    };
    let declared = spec.map(|spec| spec.roles.as_slice()).unwrap_or(&[]);
    for role in prior_roles {
        if !declared.iter().any(|entry| entry.name == role) {
            tx.execute("DELETE FROM rux_role_permissions WHERE role_name = ?1", params![role])
                .map_err(|error| error.to_string())?;
            tx.execute("DELETE FROM rux_source_roles WHERE role_name = ?1", params![role])
                .map_err(|error| error.to_string())?;
        }
    }
    for entry in declared {
        if !valid_role_name(&entry.name) || entry.permissions.iter().any(|p| !valid_permission(p)) {
            return Err(format!("Invalid source-defined role policy: {}", entry.name));
        }
        tx.execute(
            "INSERT OR IGNORE INTO rux_roles(name,created_at) VALUES (?1,?2)",
            params![entry.name, super::user_auth::unix_now()],
        ).map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM rux_role_permissions WHERE role_name = ?1", params![entry.name])
            .map_err(|error| error.to_string())?;
        for permission in &entry.permissions {
            tx.execute(
                "INSERT INTO rux_role_permissions(role_name,permission) VALUES (?1,?2)",
                params![entry.name, permission],
            ).map_err(|error| error.to_string())?;
        }
        tx.execute("INSERT OR IGNORE INTO rux_source_roles(role_name) VALUES (?1)", params![entry.name])
            .map_err(|error| error.to_string())?;
    }
    tx.commit().map_err(|error| error.to_string())
}

pub fn exists(db: &Connection, role: &str) -> rusqlite::Result<bool> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM rux_roles WHERE name = ?1)",
        params![role], |row| row.get(0))
}

pub fn create(db: &Connection, role: &str) -> Result<(), String> {
    if !valid_role_name(role) {
        return Err("Role name must be 2-64 ASCII letters, digits, '_' or '-' and start with a letter".into());
    }
    initialize(db).map_err(|error| error.to_string())?;
    db.execute("INSERT INTO rux_roles(name,created_at) VALUES (?1,?2)",
        params![role, super::user_auth::unix_now()])
        .map_err(|error| format!("Cannot create role (name may already exist): {error}"))?;
    Ok(())
}

pub fn allow(db: &Connection, role: &str, permission: &str) -> Result<(), String> {
    if !valid_permission(permission) {
        return Err("Permission must use dot-qualified names such as records.read or invoices.approve".into());
    }
    initialize(db).map_err(|e| e.to_string())?;
    if !exists(db, role).map_err(|e| e.to_string())? { return Err("Role does not exist".into()); }
    db.execute(
        "INSERT OR IGNORE INTO rux_role_permissions(role_name,permission) VALUES (?1,?2)",
        params![role,permission],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn deny(db: &Connection, role: &str, permission: &str) -> Result<bool, String> {
    if !valid_permission(permission) { return Err("Invalid permission".into()); }
    initialize(db).map_err(|e| e.to_string())?;
    let count = db.execute(
        "DELETE FROM rux_role_permissions WHERE role_name = ?1 AND permission = ?2",
        params![role,permission],
    ).map_err(|e| e.to_string())?;
    Ok(count > 0)
}

pub fn permissions(db: &Connection, role: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = db.prepare(
        "SELECT permission FROM rux_role_permissions
         WHERE role_name = ?1 ORDER BY permission"
    )?;
    let results = stmt.query_map(params![role], |row| row.get(0))?;
    results.collect()
}

pub fn role_allows(db: &Connection, role: &str, permission: &str) -> rusqlite::Result<bool> {
    if !valid_permission(permission) { return Ok(false); }
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM rux_role_permissions
         WHERE role_name = ?1 AND permission = ?2)",
        params![role,permission], |row| row.get(0),
    )
}

pub fn operation_allowed(db: &Connection, role: &str, operation: &str) -> rusqlite::Result<bool> {
    let Some(permission) = operation_permission(operation) else { return Ok(false); };
    role_allows(db, role, permission)
}

pub fn list(db: &Connection) -> Result<Vec<(String, Vec<String>)>, String> {
    initialize(db).map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT name FROM rux_roles ORDER BY name")
        .map_err(|e| e.to_string())?;
    let names = stmt.query_map([], |row| row.get::<_,String>(0))
        .map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    names.into_iter().map(|role| {
        let grants = permissions(db, &role).map_err(|e| e.to_string())?;
        Ok((role,grants))
    }).collect()
}

fn project_db(project: &Path) -> Result<Connection, String> {
    let file = project.join("records.sqlite");
    if !file.is_file() {
        return Err(format!("Database not found: {}. Start the app once.", file.display()));
    }
    let db = Connection::open(file).map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON;").map_err(|e| e.to_string())?;
    Ok(db)
}

pub fn create_in_project(project: &Path, role: &str) -> Result<(), String> {
    create(&project_db(project)?, role)
}
pub fn allow_in_project(project: &Path, role: &str, permission: &str) -> Result<(), String> {
    allow(&project_db(project)?,role,permission)
}
pub fn deny_in_project(project: &Path, role: &str, permission: &str) -> Result<bool, String> {
    deny(&project_db(project)?,role,permission)
}
pub fn list_in_project(project: &Path) -> Result<Vec<(String,Vec<String>)>,String> {
    list(&project_db(project)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_policy_grants_are_reproducible_and_revoked_when_removed() {
        let mut db = Connection::open_in_memory().unwrap();
        let spec = super::super::security_schema::SecuritySpec {
            roles: vec![super::super::security_schema::RoleDeclaration {
                name: "physician".into(), permissions: vec!["patients.read".into()],
            }],
            routes: Vec::new(),
        };
        sync_source_roles(&mut db, Some(&spec)).unwrap();
        assert!(role_allows(&db,"physician","patients.read").unwrap());
        assert!(!role_allows(&db,"physician","patients.write").unwrap());
        allow(&db,"physician","patients.write").unwrap();
        sync_source_roles(&mut db,Some(&spec)).unwrap();
        assert!(!role_allows(&db,"physician","patients.write").unwrap());
        sync_source_roles(&mut db,None).unwrap();
        assert!(!role_allows(&db,"physician","patients.read").unwrap());
        assert!(exists(&db,"physician").unwrap());
    }

    #[test]
    fn roles_are_user_defined_and_permissions_are_explicit() {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        create(&db,"doctor").unwrap();
        create(&db,"teacher").unwrap();
        assert!(!role_allows(&db,"doctor","patients.read").unwrap());
        assert!(!operation_allowed(&db,"doctor","GET").unwrap());
        allow(&db,"doctor","patients.read").unwrap();
        allow(&db,"teacher","classes.update").unwrap();
        assert!(role_allows(&db,"doctor","patients.read").unwrap());
        assert!(!role_allows(&db,"teacher","patients.read").unwrap());
        assert!(!role_allows(&db,"doctor","patients.delete").unwrap());
        assert!(deny(&db,"doctor","patients.read").unwrap());
        assert!(!role_allows(&db,"doctor","patients.read").unwrap());
        assert!(!role_allows(&db,"doctor","patients.read").unwrap());
        assert!(create(&db,"doctor").is_err());
        assert!(!valid_permission("patients.*"));
        assert!(!valid_permission("admin"));
    }

    #[test]
    fn default_roles_can_have_grants_revoked_without_regeneration() {
        let db = Connection::open_in_memory().unwrap();
        initialize(&db).unwrap();
        assert!(operation_allowed(&db,"reader","GET").unwrap());
        assert!(!operation_allowed(&db,"reader","DELETE").unwrap());
        assert!(role_allows(&db,"admin","records.access_all").unwrap());
        assert!(deny(&db,"admin","records.delete").unwrap());
        initialize(&db).unwrap();
        assert!(!operation_allowed(&db,"admin","DELETE").unwrap());
        assert!(operation_allowed(&db,"admin","GET").unwrap());
    }
}
