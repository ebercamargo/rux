use std::{net::SocketAddr, path::Path, sync::{Arc, Mutex}};
use axum::{extract::{Path as RoutePath, State}, http::StatusCode, response::{Html, IntoResponse}, routing::get, Json, Router};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<Connection>>,
    fields: Arc<Vec<(String, String)>>,
    html: Arc<String>,
}

fn validate(input: &Value, fields: &[(String, String)]) -> Result<Map<String, Value>, StatusCode> {
    let object = input.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    let mut result = Map::new();
    for (name, kind) in fields {
        let value = object.get(name).and_then(Value::as_str).ok_or(StatusCode::BAD_REQUEST)?;
        let value = value.trim();
        if value.is_empty() || value.len() > 250 { return Err(StatusCode::BAD_REQUEST); }
        let valid = match kind.as_str() {
            "Email" => {
                let parts: Vec<_> = value.split('@').collect();
                parts.len() == 2 && !parts[0].is_empty() && parts[1].contains('.') && !value.contains(char::is_whitespace)
            }
            "Int" => value.parse::<i64>().is_ok(),
            "Decimal" => value.parse::<f64>().is_ok_and(f64::is_finite),
            "Bool" => value == "true" || value == "false",
            "Date" => value.len() == 10 && value.as_bytes().get(4) == Some(&b'-') && value.as_bytes().get(7) == Some(&b'-')
                && value.chars().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()),
            _ => true,
        };
        if !valid { return Err(StatusCode::BAD_REQUEST); }
        result.insert(name.clone(), Value::String(value.to_string()));
    }
    Ok(result)
}

async fn index(State(state): State<AppState>) -> Html<String> {
    Html((*state.html).clone())
}

async fn list(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut stmt = db.prepare("SELECT id, payload FROM records ORDER BY rowid DESC")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((id, payload))
    }).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut items = Vec::new();
    for row in rows {
        let (id, payload) = row.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut value: Value = serde_json::from_str(&payload).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        value["id"] = json!(id);
        items.push(value);
    }
    Ok(Json(json!(items)))
}

async fn create(State(state): State<AppState>, Json(input): Json<Value>) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let mut payload = Value::Object(validate(&input, &state.fields)?);
    let id = next_id();
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    db.execute("INSERT INTO records (id, payload) VALUES (?1, ?2)", params![id, payload.to_string()])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    payload["id"] = json!(id);
    Ok((StatusCode::CREATED, Json(payload)))
}

async fn update(State(state): State<AppState>, RoutePath(id): RoutePath<String>, Json(input): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let mut payload = Value::Object(validate(&input, &state.fields)?);
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let count = db.execute("UPDATE records SET payload = ?1 WHERE id = ?2", params![payload.to_string(), id])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if count == 0 { return Err(StatusCode::NOT_FOUND); }
    payload["id"] = json!(id);
    Ok(Json(payload))
}

async fn delete(State(state): State<AppState>, RoutePath(id): RoutePath<String>) -> Result<impl IntoResponse, StatusCode> {
    let db = state.db.lock().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let count = db.execute("DELETE FROM records WHERE id = ?1", params![id])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if count == 0 { return Err(StatusCode::NOT_FOUND); }
    Ok(Json(json!({"deleted": true})))
}

fn next_id() -> String {
    // UUID v4 via SQLite's randomblob, without introducing a second RNG dependency.
    let id: String = Connection::open_in_memory().and_then(|db| {
        db.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
    }).unwrap_or_default();
    format!("{}-{}-4{}-a{}-{}", &id[0..8], &id[8..12], &id[13..16], &id[17..20], &id[20..32])
}

pub async fn serve(name: &str, fields: Vec<(String, String)>) -> Result<(), String> {
    let directory = Path::new(name);
    if !directory.exists() { super::crud::create_typed(name, &fields)?; }
    let html = std::fs::read_to_string(directory.join("index.html"))
        .map_err(|e| format!("cannot read generated UI: {e}"))?;
    let db = Connection::open(directory.join("records.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, payload TEXT NOT NULL);")
        .map_err(|e| e.to_string())?;
    let state = AppState { db: Arc::new(Mutex::new(db)), fields: Arc::new(fields), html: Arc::new(html) };
    let router = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/api/records", get(list).post(create))
        .route("/api/records/{id}", axum::routing::put(update).delete(delete))
        .with_state(state);
    let address = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|e| e.to_string())?;
    println!("RUX native CRUD: http://{address} (SQLite, no Node.js)");
    axum::serve(listener, router).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_typed_payloads() {
        let fields = vec![("email".into(), "Email".into()), ("age".into(), "Int".into())];
        assert!(validate(&json!({"email":"a@example.com","age":"42"}), &fields).is_ok());
        assert!(validate(&json!({"email":"wrong","age":"42"}), &fields).is_err());
        assert!(validate(&json!({"email":"a@example.com","age":"abc"}), &fields).is_err());
    }
}
