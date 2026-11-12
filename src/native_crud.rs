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
    api_token: Option<Arc<String>>,
    read_token: Option<Arc<String>>,
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

fn valid_mask(value: &str, pattern: &str) -> bool {
    let mut chars = value.chars();
    for symbol in pattern.chars() {
        let Some(actual) = chars.next() else { return false; };
        if symbol == '9' {
            if !actual.is_ascii_digit() { return false; }
        } else if actual != symbol { return false; }
    }
    chars.next().is_none()
}

fn valid_datetime(value: &str, pattern: &str) -> bool {
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    let Ok(format) = super::crud_schema::chrono_datetime_pattern(pattern) else { return false; };
    let has_date = format.contains("%d") || format.contains("%m") || format.contains("%b") || format.contains("%B") || format.contains("%y") || format.contains("%Y");
    let has_time = format.contains("%H") || format.contains("%I") || format.contains("%M") || format.contains("%S") || format.contains("%f") || format.contains("%p");
    if has_date && has_time {
        NaiveDateTime::parse_from_str(value, &format).is_ok()
    } else if has_time {
        NaiveTime::parse_from_str(value, &format).is_ok()
    } else if has_date {
        if NaiveDate::parse_from_str(value, &format).is_ok() { return true; }
        // Partial month/year formats are validated by supplying a harmless default day.
        if !format.contains("%d") && (format.contains("%m") || format.contains("%b") || format.contains("%B")) && (format.contains("%y") || format.contains("%Y")) {
            return NaiveDate::parse_from_str(&format!("{value}-01"), &format!("{format}-%d")).is_ok();
        }
        false
    } else { false }
}

fn validate(input: &Value, fields: &[(String, String)]) -> Result<Map<String, Value>, StatusCode> {
    let object = input.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    let mut result = Map::new();
    for (name, kind) in fields {
        let value = object.get(name).and_then(Value::as_str).ok_or(StatusCode::BAD_REQUEST)?;
        let value = value.trim();
        if value.is_empty() || value.len() > 250 { return Err(StatusCode::BAD_REQUEST); }
        let (kind, regex_pattern) = match kind.split_once('~') {
            Some((base, pattern)) => (base, Some(pattern)),
            None => (kind.as_str(), None),
        };
        let valid = if let Some(pattern) = kind.strip_prefix("Text|") {
            valid_mask(value, pattern)
        } else if let Some(pattern) = kind.strip_prefix("DateTime|") {
            valid_datetime(value, pattern)
        } else { match kind {
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
        }};
        if !valid || regex_pattern.is_some_and(|pattern| regex::Regex::new(pattern).map_or(true, |regex| !regex.is_match(value))) {
            return Err(StatusCode::BAD_REQUEST);
        }
        result.insert(name.clone(), Value::String(value.to_string()));
    }
    Ok(result)
}

fn token_matches(provided: &str, expected: &str) -> bool {
    let Some(value) = provided.strip_prefix("Bearer ") else { return false; };
    let a = value.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() { return false; }
    let mut diff = 0u8;
    for (&left, &right) in a.iter().zip(b) { diff |= left ^ right; }
    diff == 0
}

fn authorize_api(method: &axum::http::Method, authorization: Option<&str>, write_token: Option<&str>, read_token: Option<&str>) -> bool {
    if write_token.is_none() && read_token.is_none() { return true; }
    let Some(header) = authorization else { return false; };
    if write_token.is_some_and(|token| token_matches(header, token)) { return true; }
    (*method == axum::http::Method::GET || *method == axum::http::Method::HEAD)
        && read_token.is_some_and(|token| token_matches(header, token))
}

async fn limit_api(State(state): State<AppState>, request: Request, next: Next) -> impl IntoResponse {
    state.requests_total.fetch_add(1, Ordering::Relaxed);
    if request.uri().path().starts_with("/api/") {
        let authorization = request.headers().get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        if !authorize_api(request.method(), authorization, state.api_token.as_deref().map(String::as_str), state.read_token.as_deref().map(String::as_str)) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
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
    super::database::selected_backend(std::env::var("RUX_DATABASE_URL").ok().as_deref())
        .map_err(|error| format!("RUX_DATABASE_URL: {error}"))?;
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
    let host = std::env::var("RUX_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string())
        .parse().map_err(|_| "PORT must be a valid TCP port".to_string())?;
    let address: SocketAddr = format!("{host}:{port}").parse()
        .map_err(|_| "RUX_HOST must be an IPv4 or IPv6 address".to_string())?;
    validate_network_exposure(address.ip(), std::env::var("RUX_ALLOW_UNAUTHENTICATED_REMOTE").ok().as_deref())?;
    let api_token = std::env::var("RUX_API_TOKEN").ok().filter(|value| !value.is_empty());
    if !address.ip().is_loopback() && api_token.as_ref().is_none_or(|token| token.len() < 32) {
        return Err("Remote CRUD requires RUX_API_TOKEN with at least 32 characters. Use TLS at a trusted reverse proxy; this is not a full user authentication system.".into());
    }
    if api_token.as_ref().is_some_and(|token| token.len() < 32) {
        return Err("RUX_API_TOKEN must contain at least 32 characters".into());
    }
    let environment = std::env::var("RUX_ENV").unwrap_or_else(|_| "development".into());
    validate_runtime_environment(&environment, api_token.as_deref())?;
    let read_token = std::env::var("RUX_API_READ_TOKEN").ok().filter(|value| !value.is_empty());
    if read_token.as_ref().is_some_and(|token| token.len() < 32) {
        return Err("RUX_API_READ_TOKEN must contain at least 32 characters".into());
    }
    if read_token.is_some() && api_token.is_none() {
        return Err("RUX_API_READ_TOKEN requires RUX_API_TOKEN for write operations".into());
    }
    if read_token.as_ref().zip(api_token.as_ref()).is_some_and(|(read, write)| read == write) {
        return Err("RUX_API_READ_TOKEN must differ from RUX_API_TOKEN".into());
    }
    let accepting = Arc::new(AtomicBool::new(true));
    let state = AppState { db, fields: Arc::new(fields), html: Arc::new(html), manifest: Arc::new(manifest_content), api_token: api_token.map(Arc::new), read_token: read_token.map(Arc::new), accepting: Arc::clone(&accepting), api_slots: Arc::new(Semaphore::new(max_requests)), waiting_slots: Arc::new(Semaphore::new(max_waiting)), api_wait: Duration::from_millis(wait_ms), db_slots: Arc::new(Semaphore::new(pool_size as usize)), db_wait: Duration::from_millis(db_wait_ms), requests_total: Arc::new(AtomicU64::new(0)), rejected_total: Arc::new(AtomicU64::new(0)), request_duration_micros: Arc::new(AtomicU64::new(0)) };
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
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|e| e.to_string())?;
    println!("RUX native CRUD: http://{address} (SQLite)");
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

fn validate_runtime_environment(environment: &str, api_token: Option<&str>) -> Result<(), String> {
    match environment {
        "development" | "test" => Ok(()),
        "production" => {
            if api_token.is_none_or(|token| token.len() < 32) {
                Err("Production mode requires RUX_API_TOKEN with at least 32 characters. Configure HTTPS at the ingress or reverse proxy and implement user authentication before serving sensitive data.".into())
            } else {
                Ok(())
            }
        }
        _ => Err("RUX_ENV must be development, test, or production".into()),
    }
}

fn validate_network_exposure(ip: std::net::IpAddr, explicit_opt_in: Option<&str>) -> Result<(), String> {
    if ip.is_loopback() || explicit_opt_in == Some("true") {
        return Ok(());
    }
    Err("Refusing to expose unauthenticated CRUD outside localhost. Set RUX_ALLOW_UNAUTHENTICATED_REMOTE=true only in an isolated, trusted development environment; this does not enable authentication.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_requires_api_credentials() {
        let strong = "a".repeat(32);
        assert!(validate_runtime_environment("development", None).is_ok());
        assert!(validate_runtime_environment("test", None).is_ok());
        assert!(validate_runtime_environment("production", None).is_err());
        assert!(validate_runtime_environment("production", Some("short")).is_err());
        assert!(validate_runtime_environment("production", Some(&strong)).is_ok());
        assert!(validate_runtime_environment("prod", Some(&strong)).is_err());
    }
    #[test]
    fn accepts_flexible_date_time_formats() {
        assert!(valid_datetime("83-21-06", "yy-dd-mm"));
        assert!(valid_datetime("2026-11-12", "yyyy-MM-dd"));
        assert!(valid_datetime("12/11/2026 13:45:20", "dd/MM/yyyy HH:mm:ss"));
        assert!(valid_datetime("13:45:20", "HH:mm:ss"));
        assert!(valid_datetime("11/2026", "MM/yyyy"));
        assert!(!valid_datetime("2026-02-30", "yyyy-MM-dd"));
        assert!(!valid_datetime("83-32-06", "yy-dd-mm"));
    }
    #[test]
    fn api_tokens_enforce_read_only_access() {
        use axum::http::Method;
        let write = "w".repeat(32);
        let read = "r".repeat(32);
        let read_header = format!("Bearer {read}");
        let write_header = format!("Bearer {write}");
        assert!(authorize_api(&Method::GET, Some(&read_header), Some(&write), Some(&read)));
        assert!(authorize_api(&Method::HEAD, Some(&read_header), Some(&write), Some(&read)));
        assert!(!authorize_api(&Method::POST, Some(&read_header), Some(&write), Some(&read)));
        assert!(!authorize_api(&Method::PUT, Some(&read_header), Some(&write), Some(&read)));
        assert!(!authorize_api(&Method::DELETE, Some(&read_header), Some(&write), Some(&read)));
        assert!(authorize_api(&Method::DELETE, Some(&write_header), Some(&write), Some(&read)));
        assert!(!authorize_api(&Method::GET, None, Some(&write), Some(&read)));
        assert!(authorize_api(&Method::GET, None, None, None));
    }
    #[test]
    fn bearer_token_requires_exact_match() {
        let token = "a".repeat(32);
        assert!(token_matches(&format!("Bearer {token}"), &token));
        assert!(!token_matches(&format!("Bearer {}b", &token[..31]), &token));
        assert!(!token_matches(&token, &token));
        assert!(!token_matches("Bearer wrong", &token));
    }
    #[test]
    fn remote_crud_requires_explicit_opt_in() {
        assert!(validate_network_exposure("127.0.0.1".parse().unwrap(), None).is_ok());
        assert!(validate_network_exposure("::1".parse().unwrap(), None).is_ok());
        assert!(validate_network_exposure("0.0.0.0".parse().unwrap(), None).is_err());
        assert!(validate_network_exposure("192.168.1.10".parse().unwrap(), Some("false")).is_err());
        assert!(validate_network_exposure("0.0.0.0".parse().unwrap(), Some("true")).is_ok());
    }
    #[tokio::test]
    async fn db_admission_rejects_when_saturated() {
        let state = AppState {
            db: r2d2::Pool::builder().max_size(1).build(SqliteConnectionManager::memory()).unwrap(),
            fields: Arc::new(vec![]),
            html: Arc::new(String::new()),
            manifest: Arc::new(String::new()),
            api_token: None,
            read_token: None,
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
            let rows = statement.query_map(params![i64::MAX, 50, q, field], |row| row.get::<_, String>(1))
                .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            rows.len()
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
