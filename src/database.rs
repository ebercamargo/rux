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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_database_drivers() {
        assert_eq!(DatabaseKind::parse("PostgreSQL"), DatabaseKind::Postgres);
        assert_eq!(DatabaseKind::parse("MSSQL"), DatabaseKind::SqlServer);
        assert_eq!(DatabaseKind::parse("db2"), DatabaseKind::Db2);
        assert_eq!(DatabaseKind::parse("custom-db"), DatabaseKind::Custom("custom-db".into()));
    }
}
