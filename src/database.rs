//! Database-neutral contract for RUX applications.
//! Backends implement this contract; no SQL dialect is assumed by the language.
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseKind {
    Sqlite,
    Postgres,
    Mysql,
    Mariadb,
    SqlServer,
    Oracle,
    Db2,
    Custom(String),
}

impl DatabaseKind {
    pub fn parse(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "sqlite" => Self::Sqlite,
            "postgres" | "postgresql" => Self::Postgres,
            "mysql" => Self::Mysql,
            "mariadb" => Self::Mariadb,
            "sqlserver" | "mssql" => Self::SqlServer,
            "oracle" => Self::Oracle,
            "db2" => Self::Db2,
            other => Self::Custom(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageRequest {
    pub limit: u32,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PageResult {
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
}

#[derive(Debug)]
pub enum DatabaseError {
    Unavailable(String),
    InvalidData(String),
    NotFound,
    Internal(String),
}

/// Implementations must enforce bounded pagination and parameterized queries.
/// The runtime may invoke implementations concurrently.
pub trait CrudRepository: Send + Sync {
    fn list(&self, page: PageRequest) -> Result<PageResult, DatabaseError>;
    fn create(&self, values: Value) -> Result<Value, DatabaseError>;
    fn update(&self, id: &str, values: Value) -> Result<Value, DatabaseError>;
    fn delete(&self, id: &str) -> Result<(), DatabaseError>;
    fn ready(&self) -> Result<(), DatabaseError>;
}


impl DatabaseKind {
    pub fn from_url(url: &str) -> Result<Self, DatabaseError> {
        let scheme = url.split_once("://")
            .map(|(scheme, _)| scheme)
            .ok_or_else(|| DatabaseError::InvalidData("database URL must contain ://".into()))?;
        let kind = match scheme.to_ascii_lowercase().as_str() {
            "sqlite" => Self::Sqlite,
            "postgres" | "postgresql" => Self::Postgres,
            "mysql" => Self::Mysql,
            "mariadb" => Self::Mariadb,
            "sqlserver" | "mssql" => Self::SqlServer,
            "oracle" => Self::Oracle,
            "db2" => Self::Db2,
            _ => return Err(DatabaseError::InvalidData("unsupported database URL scheme".into())),
        };
        Ok(kind)
    }

    pub fn is_implemented(&self) -> bool {
        matches!(self, Self::Sqlite)
    }
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::InvalidData(message) | Self::Internal(message) =>
                formatter.write_str(message),
            Self::NotFound => formatter.write_str("record not found"),
        }
    }
}

impl std::error::Error for DatabaseError {}

/// The backend selection is deliberately strict: no silent fallback to SQLite
/// when a user has configured a different database.
pub fn selected_backend(url: Option<&str>) -> Result<DatabaseKind, DatabaseError> {
    let kind = match url {
        Some(value) => DatabaseKind::from_url(value)?,
        None => DatabaseKind::Sqlite,
    };
    if !kind.is_implemented() {
        return Err(DatabaseError::Unavailable(format!(
            "database backend {kind:?} is recognized but not implemented yet"
        )));
    }
    Ok(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_sqlite_by_default_and_refuses_unsupported_backends() {
        assert_eq!(selected_backend(None).unwrap(), DatabaseKind::Sqlite);
        assert_eq!(selected_backend(Some("sqlite://local.db")).unwrap(), DatabaseKind::Sqlite);
        assert!(matches!(selected_backend(Some("postgres://localhost/rux")), Err(DatabaseError::Unavailable(_))));
        assert!(matches!(selected_backend(Some("unknown://localhost/rux")), Err(DatabaseError::InvalidData(_))));
        assert!(matches!(selected_backend(Some("postgres")), Err(DatabaseError::InvalidData(_))));
    }

    #[test]
    fn recognizes_database_drivers() {
        assert_eq!(DatabaseKind::parse("PostgreSQL"), DatabaseKind::Postgres);
        assert_eq!(DatabaseKind::parse("MSSQL"), DatabaseKind::SqlServer);
        assert_eq!(DatabaseKind::parse("db2"), DatabaseKind::Db2);
        assert_eq!(DatabaseKind::parse("custom-db"), DatabaseKind::Custom("custom-db".into()));
    }
}
