//! PostgreSQL-backed CRUD storage. The HTTP routing integration is incremental.
use postgres::NoTls;
use r2d2::Pool;
use r2d2_postgres::PostgresConnectionManager;
use serde_json::{json, Value};

use crate::database::{CrudRepository, DatabaseError, PageRequest, PageResult};

pub struct PostgresRepository {
    pool: Pool<PostgresConnectionManager<NoTls>>,
}

impl PostgresRepository {
    pub fn connect(url: &str, size: u32) -> Result<Self, DatabaseError> {
        if !(1..=128).contains(&size) {
            return Err(DatabaseError::InvalidData("pool size must be between 1 and 128".into()));
        }
        let config = url.parse::<postgres::Config>()
            .map_err(|_| DatabaseError::InvalidData("invalid PostgreSQL connection URL".into()))?;
        let manager = PostgresConnectionManager::new(config, NoTls);
        let pool = Pool::builder().max_size(size).build(manager)
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        let repository = Self { pool };
        repository.initialize()?;
        Ok(repository)
    }

    fn initialize(&self) -> Result<(), DatabaseError> {
        let mut connection = self.pool.get()
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        connection.batch_execute(
            "CREATE TABLE IF NOT EXISTS rux_records (
                sequence BIGSERIAL PRIMARY KEY,
                id TEXT NOT NULL UNIQUE,
                payload JSONB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS rux_records_id_idx ON rux_records(id);"
        ).map_err(|e| DatabaseError::Internal(e.to_string()))
    }
}

impl CrudRepository for PostgresRepository {
    fn ready(&self) -> Result<(), DatabaseError> {
        let mut connection = self.pool.get()
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        connection.query_one("SELECT 1", &[])
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        Ok(())
    }

    fn list(&self, page: PageRequest) -> Result<PageResult, DatabaseError> {
        let limit = i64::from(page.limit.clamp(1, 100));
        let before = match page.cursor {
            Some(cursor) => cursor.parse::<i64>()
                .map_err(|_| DatabaseError::InvalidData("invalid pagination cursor".into()))?,
            None => i64::MAX,
        };
        let mut connection = self.pool.get()
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        let rows = connection.query(
            "SELECT sequence, id, payload::text FROM rux_records
             WHERE sequence < $1 ORDER BY sequence DESC LIMIT $2",
            &[&before, &limit]
        ).map_err(|e| DatabaseError::Internal(e.to_string()))?;
        let mut items = Vec::with_capacity(rows.len());
        let mut last = None;
        for row in rows {
            let sequence: i64 = row.get(0);
            let id: String = row.get(1);
            let payload: String = row.get(2);
            let mut value: Value = serde_json::from_str(&payload)
                .map_err(|e| DatabaseError::Internal(e.to_string()))?;
            value["id"] = json!(id);
            value["_cursor"] = json!(sequence);
            last = Some(sequence.to_string());
            items.push(value);
        }
        let next_cursor = if items.len() == limit as usize { last } else { None };
        Ok(PageResult { items, next_cursor })
    }

    fn create(&self, values: Value) -> Result<Value, DatabaseError> {
        if !values.is_object() {
            return Err(DatabaseError::InvalidData("record must be an object".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let payload = values.to_string();
        let mut connection = self.pool.get()
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        connection.execute(
            "INSERT INTO rux_records (id, payload) VALUES ($1, $2::jsonb)",
            &[&id, &payload]
        ).map_err(|e| DatabaseError::Internal(e.to_string()))?;
        let mut result = values;
        result["id"] = json!(id);
        Ok(result)
    }

    fn update(&self, id: &str, values: Value) -> Result<Value, DatabaseError> {
        if !values.is_object() {
            return Err(DatabaseError::InvalidData("record must be an object".into()));
        }
        let payload = values.to_string();
        let mut connection = self.pool.get()
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        let changed = connection.execute(
            "UPDATE rux_records SET payload = $1::jsonb WHERE id = $2",
            &[&payload, &id]
        ).map_err(|e| DatabaseError::Internal(e.to_string()))?;
        if changed == 0 { return Err(DatabaseError::NotFound); }
        let mut result = values;
        result["id"] = json!(id);
        Ok(result)
    }

    fn delete(&self, id: &str) -> Result<(), DatabaseError> {
        let mut connection = self.pool.get()
            .map_err(|e| DatabaseError::Unavailable(e.to_string()))?;
        let changed = connection.execute("DELETE FROM rux_records WHERE id = $1", &[&id])
            .map_err(|e| DatabaseError::Internal(e.to_string()))?;
        if changed == 0 { return Err(DatabaseError::NotFound); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_connection_configuration() {
        assert!(matches!(
            PostgresRepository::connect("postgres://localhost/rux", 0),
            Err(DatabaseError::InvalidData(_))
        ));
        assert!(matches!(
            PostgresRepository::connect("not a URL", 1),
            Err(DatabaseError::InvalidData(_))
        ));
    }
}
