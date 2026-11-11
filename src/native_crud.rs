use std::{net::SocketAddr, path::Path, sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc}};
use axum::{extract::{Path as RoutePath, Query, State}, http::StatusCode, response::{Html, IntoResponse}, routing::get, Json, Router};
use rusqlite::params;
use r2d2_sqlite::SqliteConnectionManager;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use std::time::{Duration, Instant};
use axum::{middleware::{self, Next}, extract::Request};

#[derive(Clone)]
struct AppState {
    db: r2d2::Pool<SqliteConnectionManager>,
    fields: Arc<Vec<(String, String)>>,
    html: Arc<String>,
    manifest: Arc<String>,
    accepting: Arc<AtomicBool>,
    api_slots: Arc<Semaphore>,
    waiting_slots: Arc<Semaphore>,
    api_wait: Duration,
    db_slots: Arc<Semaphore>,
    db_wait: Duration,
    requests_total: Arc<AtomicU64>,
    rejected_total: Arc<AtomicU64>,
    request_duration_micros: Arc<AtomicU64>,
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

async fn limit_api(State(state): State<AppState>, request: Request, next: Next) -> impl IntoResponse {
    state.requests_total.fetch_add(1, Ordering::Relaxed);
    if !state.accepting.load(Ordering::Acquire) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let permit = if let Ok(permit) = state.api_slots.clone().try_acquire_owned() {
        permit
    } else {
        let waiting = match state.waiting_slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                state.rejected_total.fetch_add(1, Ordering::Relaxed);
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
        };
        let result = tokio::time::timeout(state.api_wait, state.api_slots.clone().acquire_owned()).await;
        drop(waiting);
        match result {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => return Err(StatusCode::SERVICE_UNAVAILABLE),
            Err(_) => {
                state.rejected_total.fetch_add(1, Ordering::Relaxed);
                return Err(StatusCode::TOO_MANY_REQUESTS);
            },
        }
    };
    let start = Instant::now();
    let response = next.run(request).await;
    state.request_duration_micros.fetch_add(start.elapsed().as_micros().min(u64::MAX as u128) as u64, Ordering::Relaxed);
    drop(permit);
    Ok(response)
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "requests_total": state.requests_total.load(Ordering::Relaxed),
        "rejected_total": state.rejected_total.load(Ordering::Relaxed),
        "request_duration_micros_total": state.request_duration_micros.load(Ordering::Relaxed),
        "api_slots_available": state.api_slots.available_permits(),
        "queued_slots_available": state.waiting_slots.available_permits()
    }))
}

async fn health_live() -> impl IntoResponse {
    Json(json!({"status": "UP"}))
}

async fn acquire_db(state: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, StatusCode> {
    tokio::time::timeout(state.db_wait, state.db_slots.clone().acquire_owned())
        .await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

async fn health_ready(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    if !state.accepting.load(Ordering::Acquire) { return Err(StatusCode::SERVICE_UNAVAILABLE); }
    let _db_permit = acquire_db(&state).await?;
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

const LIST_SQL: &str = "SELECT rowid, id, payload FROM records
             WHERE rowid < ?1
               AND (?3 = '' OR EXISTS (
                 SELECT 1 FROM json_each(payload)
                 WHERE (?4 = '' OR json_each.key = ?4)
                   AND instr(lower(CAST(json_each.value AS TEXT)), lower(?3)) > 0
               ))
             ORDER BY rowid DESC LIMIT ?2";

#[derive(serde::Deserialize)]
struct Page {
    limit: Option<u32>,
    before: Option<i64>,
    q: Option<String>,
    field: Option<String>,
}

async fn list(State(state): State<AppState>, Query(page): Query<Page>) -> Result<Json<Value>, StatusCode> {
    let limit = page.limit.unwrap_or(50).clamp(1, 100);
    let before = page.before.unwrap_or(i64::MAX);
    let query = page.q.unwrap_or_default();
    if query.len() > 250 { return Err(StatusCode::BAD_REQUEST); }
    let field = page.field.unwrap_or_default();
    if !field.is_empty() && !state.fields.iter().any(|(name, _)| name == &field) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let _db_permit = acquire_db(&state).await?;
    tokio::task::spawn_blocking(move || {
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let mut stmt = db.prepare(LIST_SQL)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let rows = stmt.query_map(params![before, limit, query, field], |row| {
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
    let _db_permit = acquire_db(&state).await?;
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
    let _db_permit = acquire_db(&state).await?;
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
    let _db_permit = acquire_db(&state).await?;
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
    if !directory.exists() {
        super::crud::create_typed(name, &fields)?;
    } else {
        super::crud::refresh_typed_ui(name, &fields)?;
    }
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
    let max_requests: usize = std::env::var("RUX_MAX_CONCURRENT_REQUESTS")
        .unwrap_or_else(|_| "256".into())
        .parse().map_err(|_| "RUX_MAX_CONCURRENT_REQUESTS must be an integer".to_string())?;
    if !(1..=100_000).contains(&max_requests) {
        return Err("RUX_MAX_CONCURRENT_REQUESTS must be between 1 and 100000".into());
    }
    let max_waiting: usize = std::env::var("RUX_MAX_QUEUED_REQUESTS")
        .unwrap_or_else(|_| "512".into())
        .parse().map_err(|_| "RUX_MAX_QUEUED_REQUESTS must be an integer".to_string())?;
    if max_waiting > 100_000 {
        return Err("RUX_MAX_QUEUED_REQUESTS must be between 0 and 100000".into());
    }
    let wait_ms: u64 = std::env::var("RUX_API_QUEUE_WAIT_MS")
        .unwrap_or_else(|_| "50".into())
        .parse().map_err(|_| "RUX_API_QUEUE_WAIT_MS must be an integer".to_string())?;
    if wait_ms > 10_000 {
        return Err("RUX_API_QUEUE_WAIT_MS must be between 0 and 10000".into());
    }
    let db_wait_ms: u64 = std::env::var("RUX_DB_WAIT_MS")
        .unwrap_or_else(|_| "2000".into())
        .parse().map_err(|_| "RUX_DB_WAIT_MS must be an integer".to_string())?;
    if db_wait_ms > 60_000 {
        return Err("RUX_DB_WAIT_MS must be between 0 and 60000".into());
    }
    let accepting = Arc::new(AtomicBool::new(true));
    let state = AppState { db, fields: Arc::new(fields), html: Arc::new(html), manifest: Arc::new(manifest_content), accepting: Arc::clone(&accepting), api_slots: Arc::new(Semaphore::new(max_requests)), waiting_slots: Arc::new(Semaphore::new(max_waiting)), api_wait: Duration::from_millis(wait_ms), db_slots: Arc::new(Semaphore::new(pool_size as usize)), db_wait: Duration::from_millis(db_wait_ms), requests_total: Arc::new(AtomicU64::new(0)), rejected_total: Arc::new(AtomicU64::new(0)), request_duration_micros: Arc::new(AtomicU64::new(0)) };
    let router = Router::new()
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/manifest.webmanifest", get(manifest))
        .route("/api/records", get(list).post(create))
        .route("/api/records/{id}", axum::routing::put(update).delete(delete))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit_api))
        .with_state(state.clone());
    let router = if std::env::var("RUX_EXPOSE_METRICS").as_deref() == Ok("true") {
        router.route("/metrics", get(metrics).with_state(state))
    } else {
        router
    };
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
    #[tokio::test]
    async fn db_admission_rejects_when_saturated() {
        let state = AppState {
            db: r2d2::Pool::builder().max_size(1).build(SqliteConnectionManager::memory()).unwrap(),
            fields: Arc::new(vec![]),
            html: Arc::new(String::new()),
            manifest: Arc::new(String::new()),
            accepting: Arc::new(AtomicBool::new(true)),
            api_slots: Arc::new(Semaphore::new(1)),
            waiting_slots: Arc::new(Semaphore::new(1)),
            api_wait: Duration::from_millis(1),
            db_slots: Arc::new(Semaphore::new(1)),
            db_wait: Duration::from_millis(1),
            requests_total: Arc::new(AtomicU64::new(0)),
            rejected_total: Arc::new(AtomicU64::new(0)),
            request_duration_micros: Arc::new(AtomicU64::new(0)),
        };
        let permit = acquire_db(&state).await.unwrap();
        assert!(matches!(acquire_db(&state).await, Err(StatusCode::SERVICE_UNAVAILABLE)));
        drop(permit);
        assert!(acquire_db(&state).await.is_ok());
    }

    #[test]
    fn search_uses_bound_parameters_and_resists_sql_injection() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE records (id TEXT PRIMARY KEY, payload TEXT NOT NULL);").unwrap();
        db.execute("INSERT INTO records (id, payload) VALUES (?1, ?2)",
            params!["one", r#"{"nome":"Alice","email":"alice@example.com"}"#]).unwrap();
        db.execute("INSERT INTO records (id, payload) VALUES (?1, ?2)",
            params!["two", r#"{"nome":"Bob","email":"bob@example.com"}"#]).unwrap();
        let count = |q: &str, field: &str| -> usize {
            let mut statement = db.prepare(LIST_SQL).unwrap();
            statement.query_map(params![i64::MAX, 50, q, field], |row| row.get::<_, String>(1))
                .unwrap().collect::<Result<Vec<_>, _>>().unwrap().len()
        };
        assert_eq!(count("Alice", ""), 1);
        assert_eq!(count("alice@", "email"), 1);
        assert_eq!(count("alice@", "nome"), 0);
        assert_eq!(count("' OR 1=1 --", ""), 0);
        assert_eq!(count("x'); DROP TABLE records; --", ""), 0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM records", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn validates_typed_payloads() {
        let fields = vec![("email".into(), "Email".into()), ("age".into(), "Int".into())];
        assert!(validate(&json!({"email":"a@example.com","age":"42"}), &fields).is_ok());
        assert!(validate(&json!({"email":"wrong","age":"42"}), &fields).is_err());
        assert!(validate(&json!({"email":"a@example.com","age":"abc"}), &fields).is_err());
    }
}
