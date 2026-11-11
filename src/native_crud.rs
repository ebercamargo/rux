use std::{net::SocketAddr, path::Path, sync::{atomic::{AtomicBool, Ordering}, Arc}};
use axum::{extract::{Path as RoutePath, Query, State}, http::StatusCode, response::{Html, IntoResponse}, routing::get, Json, Router};
use rusqlite::params;
use r2d2_sqlite::SqliteConnectionManager;
use serde_json::{json, Map, Value};

#[derive(Clone)]
struct AppState {
    db: r2d2::Pool<SqliteConnectionManager>,
    fields: Arc<Vec<(String, String)>>,
    html: Arc<String>,
    manifest: Arc<String>,
    accepting: Arc<AtomicBool>,
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

async fn health_live() -> impl IntoResponse {
    Json(json!({"status": "UP"}))
}

async fn health_ready(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    if !state.accepting.load(Ordering::Acquire) { return Err(StatusCode::SERVICE_UNAVAILABLE); }
    tokio::task::spawn_blocking(move || {
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        db.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Ok(Json(json!({"status": "UP"})))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

async fn index(State(state): State<AppState>) -> Html<String> {
    Html((*state.html).clone())
}

async fn manifest(State(state): State<AppState>) -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "application/manifest+json")], (*state.manifest).clone())
}

#[derive(serde::Deserialize)]
struct Page {
    limit: Option<u32>,
    before: Option<i64>,
}

async fn list(State(state): State<AppState>, Query(page): Query<Page>) -> Result<Json<Value>, StatusCode> {
    let limit = page.limit.unwrap_or(50).clamp(1, 100);
    let before = page.before.unwrap_or(i64::MAX);
    tokio::task::spawn_blocking(move || {
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let mut stmt = db.prepare("SELECT rowid, id, payload FROM records WHERE rowid < ?1 ORDER BY rowid DESC LIMIT ?2")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let rows = stmt.query_map(params![before, limit], |row| {
            let cursor: i64 = row.get(0)?;
            let id: String = row.get(1)?;
            let payload: String = row.get(2)?;
            Ok((cursor, id, payload))
        }).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut items = Vec::new();
        for row in rows {
            let (cursor, id, payload) = row.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let mut value: Value = serde_json::from_str(&payload).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            value["id"] = json!(id);
            value["_cursor"] = json!(cursor);
            items.push(value);
        }
        Ok(Json(json!(items)))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

async fn create(State(state): State<AppState>, Json(input): Json<Value>) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let mut payload = Value::Object(validate(&input, &state.fields)?);
    tokio::task::spawn_blocking(move || {
        let id = uuid::Uuid::new_v4().to_string();
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        db.execute("INSERT INTO records (id, payload) VALUES (?1, ?2)", params![id, payload.to_string()])
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        payload["id"] = json!(id);
        Ok((StatusCode::CREATED, Json(payload)))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

async fn update(State(state): State<AppState>, RoutePath(id): RoutePath<String>, Json(input): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let mut payload = Value::Object(validate(&input, &state.fields)?);
    tokio::task::spawn_blocking(move || {
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let count = db.execute("UPDATE records SET payload = ?1 WHERE id = ?2", params![payload.to_string(), id])
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if count == 0 { return Err(StatusCode::NOT_FOUND); }
        payload["id"] = json!(id);
        Ok(Json(payload))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

async fn delete(State(state): State<AppState>, RoutePath(id): RoutePath<String>) -> Result<impl IntoResponse, StatusCode> {
    tokio::task::spawn_blocking(move || {
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let count = db.execute("DELETE FROM records WHERE id = ?1", params![id])
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if count == 0 { return Err(StatusCode::NOT_FOUND); }
        Ok(Json(json!({"deleted": true})))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

pub async fn serve(name: &str, fields: Vec<(String, String)>) -> Result<(), String> {
    let directory = Path::new(name);
    if !directory.exists() { super::crud::create_typed(name, &fields)?; }
    let html = std::fs::read_to_string(directory.join("index.html"))
        .map_err(|e| format!("cannot read generated UI: {e}"))?;
    let manifest_content = std::fs::read_to_string(directory.join("manifest.webmanifest"))
        .map_err(|e| format!("cannot read manifest: {e}"))?;
    let manager = SqliteConnectionManager::file(directory.join("records.sqlite"))
        .with_init(|connection| connection.execute_batch("PRAGMA busy_timeout=5000;"));
    let pool_size: u32 = std::env::var("RUX_DB_POOL_SIZE")
        .unwrap_or_else(|_| "8".into())
        .parse().map_err(|_| "RUX_DB_POOL_SIZE must be an integer".to_string())?;
    if !(1..=128).contains(&pool_size) {
        return Err("RUX_DB_POOL_SIZE must be between 1 and 128".into());
    }
    let db = r2d2::Pool::builder().max_size(pool_size).build(manager).map_err(|e| e.to_string())?;
    db.get().map_err(|e| e.to_string())?
        .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS records (id TEXT PRIMARY KEY, payload TEXT NOT NULL);")
        .map_err(|e| e.to_string())?;
    let accepting = Arc::new(AtomicBool::new(true));
    let state = AppState { db, fields: Arc::new(fields), html: Arc::new(html), manifest: Arc::new(manifest_content), accepting: Arc::clone(&accepting) };
    let router = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/manifest.webmanifest", get(manifest))
        .route("/api/records", get(list).post(create))
        .route("/api/records/{id}", axum::routing::put(update).delete(delete))
        .with_state(state);
    let host = std::env::var("RUX_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string())
        .parse().map_err(|_| "PORT must be a valid TCP port".to_string())?;
    let address: SocketAddr = format!("{host}:{port}").parse()
        .map_err(|_| "RUX_HOST must be an IPv4 or IPv6 address".to_string())?;
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|e| e.to_string())?;
    println!("RUX native CRUD: http://{address} (SQLite, no Node.js)");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                if let Ok(mut terminate) = signal(SignalKind::terminate()) {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {},
                        _ = terminate.recv() => {},
                    }
                } else {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
            #[cfg(not(unix))]
            { let _ = tokio::signal::ctrl_c().await; }
            accepting.store(false, Ordering::Release);
        })
        .await.map_err(|e| e.to_string())
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
