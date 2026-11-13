use std::{collections::HashMap, net::{IpAddr, SocketAddr}, path::Path, sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc, Mutex}};
use axum::{extract::{Path as RoutePath, Query, State, DefaultBodyLimit, ConnectInfo}, Extension, http::{StatusCode, HeaderMap}, response::{Html, IntoResponse}, routing::{get, post}, Json, Router};
use rusqlite::params;
use r2d2_sqlite::SqliteConnectionManager;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use std::time::{Duration, Instant};
use axum::{middleware::{self, Next}, extract::Request};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthMode { Legacy, Identities, Users }

impl AuthMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "legacy" => Ok(Self::Legacy),
            "identities" => Ok(Self::Identities),
            "users" => Ok(Self::Users),
            _ => Err("RUX_AUTH_MODE must be legacy, identities, or users".into()),
        }
    }
}

#[derive(Clone)]
struct AppState {
    db: r2d2::Pool<SqliteConnectionManager>,
    fields: Arc<Vec<(String, String)>>,
    html: Arc<String>,
    manifest: Arc<String>,
    csp_header: Arc<String>,
    api_token: Option<Arc<String>>,
    read_token: Option<Arc<String>>,
    auth_mode: AuthMode,
    tenancy_enabled: bool,
    secure_cookie: bool,
    allowed_hosts: Arc<Vec<String>>,
    rate_windows: Arc<Mutex<HashMap<IpAddr, ClientWindow>>>,
    rate_limit_per_minute: u32,
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

#[derive(Clone, Copy)]
struct ClientWindow {
    started: Instant,
    used: u32,
}

fn within_rate_limit(windows: &Mutex<HashMap<IpAddr, ClientWindow>>, ip: IpAddr, limit: u32, now: Instant) -> bool {
    let Ok(mut windows) = windows.lock() else { return false; };
    let interval = Duration::from_secs(60);
    if let Some(client) = windows.get_mut(&ip) {
        if now.duration_since(client.started) >= interval {
            client.started = now;
            client.used = 1;
            return true;
        }
        if client.used >= limit { return false; }
        client.used += 1;
        return true;
    }
    if windows.len() >= 10_000 {
        windows.retain(|_, client| now.duration_since(client.started) < interval);
        if windows.len() >= 10_000 { return false; }
    }
    windows.insert(ip, ClientWindow { started: now, used: 1 });
    true
}

async fn rate_gate(State(state): State<AppState>, request: Request, next: Next) -> Result<axum::response::Response, StatusCode> {
    let ip = request.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !within_rate_limit(&state.rate_windows, ip, state.rate_limit_per_minute, Instant::now()) {
        state.rejected_total.fetch_add(1, Ordering::Relaxed);
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    Ok(next.run(request).await)
}

fn normalized_host(value: &str) -> Option<String> {
    if value.contains('@') { return None; }
    let authority: axum::http::uri::Authority = value.parse().ok()?;
    let host = authority.host().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if host.is_empty() { None } else { Some(host) }
}

fn allowed_host_names(ip: IpAddr, configured: Option<&str>) -> Result<Vec<String>, String> {
    let raw = match configured {
        Some(value) => value,
        None if ip.is_loopback() => return Ok(vec!["localhost".into(), "127.0.0.1".into(), "::1".into()]),
        None => return Err("Remote bind requires RUX_ALLOWED_HOSTS (comma-separated hostnames or IPs; wildcards are not supported)".into()),
    };
    if raw.len() > 4096 { return Err("RUX_ALLOWED_HOSTS is too long".into()); }
    let mut result = Vec::new();
    for item in raw.split(',') {
        let name = item.trim();
        if name.is_empty() || name == "*" || name.contains('/') || name.contains(' ') {
            return Err("RUX_ALLOWED_HOSTS contains an invalid host".into());
        }
        let authority: axum::http::uri::Authority = name.parse()
            .map_err(|_| format!("Invalid RUX_ALLOWED_HOSTS entry: {name}"))?;
        if authority.port_u16().is_some() || name.contains('@') {
            return Err("RUX_ALLOWED_HOSTS entries must not include ports or userinfo".into());
        }
        let normalized = normalized_host(name)
            .ok_or_else(|| format!("Invalid RUX_ALLOWED_HOSTS entry: {name}"))?;
        if !result.contains(&normalized) { result.push(normalized); }
        if result.len() > 32 { return Err("RUX_ALLOWED_HOSTS supports at most 32 distinct hosts".into()); }
    }
    Ok(result)
}

fn same_origin(origin: &str, host: &str) -> bool {
    let Ok(uri) = origin.parse::<axum::http::Uri>() else { return false; };
    matches!(uri.scheme_str(), Some("http" | "https"))
        && uri.authority().is_some_and(|authority| authority.as_str().eq_ignore_ascii_case(host))
}

async fn host_origin_guard(State(state): State<AppState>, request: Request, next: Next)
    -> Result<axum::response::Response, StatusCode>
{
    use axum::http::header::{HOST, ORIGIN};
    let host_header = request.headers().get(HOST)
        .and_then(|header| header.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let host = normalized_host(host_header).ok_or(StatusCode::BAD_REQUEST)?;
    if !state.allowed_hosts.iter().any(|allowed| allowed == &host) {
        return Err(StatusCode::MISDIRECTED_REQUEST);
    }
    if request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD {
        if let Some(origin) = request.headers().get(ORIGIN) {
            let origin = origin.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
            if !same_origin(origin, host_header) { return Err(StatusCode::FORBIDDEN); }
        }
    }
    Ok(next.run(request).await)
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


const SESSION_COOKIE: &str = "rux_session";

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get_all(axum::http::header::COOKIE);
    let mut tokens = cookies.iter()
        .filter_map(|header| header.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .filter(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.to_owned());
    let first = tokens.next()?;
    if tokens.next().is_some() || first.len() != 69 || !first.starts_with("ruxs_") {
        return None;
    }
    Some(first)
}

async fn browser_session(state: &AppState, headers: &HeaderMap)
    -> Result<(String, super::user_auth::Session), StatusCode>
{
    let token = session_cookie(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let _db_permit = acquire_db(state).await?;
    let pool = state.db.clone();
    let lookup = token.clone();
    let session = tokio::task::spawn_blocking(move || {
        let db = pool.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        super::user_auth::session(&db, &lookup, super::user_auth::unix_now())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
    Ok((token, session.ok_or(StatusCode::UNAUTHORIZED)?))
}

async fn check_browser(state: &AppState, headers: &HeaderMap, operation: &str, check_global_role: bool)
    -> Result<super::user_auth::Session, StatusCode>
{
    let (_, session) = browser_session(state, headers).await?;
    if check_global_role && !super::user_auth::can(&session.role, operation) {
        return Err(StatusCode::FORBIDDEN);
    }
    if !matches!(operation, "GET" | "HEAD" | "METRICS") {
        let csrf = headers.get("x-rux-csrf").and_then(|header| header.to_str().ok())
            .ok_or(StatusCode::FORBIDDEN)?;
        if !super::user_auth::csrf_matches(&session.csrf, csrf) {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(session)
}

#[derive(serde::Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}

async fn browser_login(State(state): State<AppState>, Json(form): Json<LoginForm>)
    -> Result<(HeaderMap, Json<Value>), StatusCode>
{
    if state.auth_mode != AuthMode::Users { return Err(StatusCode::NOT_FOUND); }
    if form.username.len() > 64 || form.password.len() > 128 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _db_permit = acquire_db(&state).await?;
    let pool = state.db.clone();
    let login = tokio::task::spawn_blocking(move || {
        let db = pool.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        super::user_auth::login(&db, &form.username, &form.password, super::user_auth::unix_now())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let secure = if state.secure_cookie { "; Secure" } else { "" };
    let cookie = format!("{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        SESSION_COOKIE, login.token, super::user_auth::SESSION_SECONDS, secure);
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    Ok((headers, Json(json!({
        "username": login.session.username,
        "role": login.session.role,
        "expiresAt": login.session.expires_at,
        "csrf": login.session.csrf,
    }))))
}

async fn browser_me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    if state.auth_mode != AuthMode::Users { return Err(StatusCode::NOT_FOUND); }
    let (_, session) = browser_session(&state, &headers).await?;
    Ok(Json(json!({
        "username": session.username, "role": session.role,
        "expiresAt": session.expires_at, "csrf": session.csrf
    })))
}

async fn browser_logout(State(state): State<AppState>, headers: HeaderMap)
    -> Result<(HeaderMap, Json<Value>), StatusCode>
{
    if state.auth_mode != AuthMode::Users { return Err(StatusCode::NOT_FOUND); }
    let (token, session) = browser_session(&state, &headers).await?;
    let csrf = headers.get("x-rux-csrf").and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::FORBIDDEN)?;
    if !super::user_auth::csrf_matches(&session.csrf, csrf) { return Err(StatusCode::FORBIDDEN); }
    let _db_permit = acquire_db(&state).await?;
    let pool = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let db = pool.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        super::user_auth::logout(&db, &token).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
    let secure = if state.secure_cookie { "; Secure" } else { "" };
    let cookie = format!("{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}", SESSION_COOKIE, secure);
    let mut response_headers = HeaderMap::new();
    response_headers.insert(axum::http::header::SET_COOKIE,
        axum::http::HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    Ok((response_headers, Json(json!({"loggedOut":true}))))
}

async fn check_identity(state: &AppState, authorization: Option<&str>, operation: &str) -> Result<(), StatusCode> {
    let _db_permit = acquire_db(state).await?;
    let pool = state.db.clone();
    let bearer = authorization.map(str::to_owned);
    let operation = operation.to_owned();
    let decision = tokio::task::spawn_blocking(move || {
        let db = pool.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        super::identity::authorize(&db, bearer.as_deref(), &operation)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
    match decision {
        super::identity::Decision::Allowed => Ok(()),
        super::identity::Decision::Unauthenticated => Err(StatusCode::UNAUTHORIZED),
        super::identity::Decision::Forbidden => Err(StatusCode::FORBIDDEN),
    }
}


#[derive(Clone, Debug)]
struct TenantContext {
    id: String,
}

async fn verified_tenant(state: &AppState, user_id: &str, id: &str)
    -> Result<super::tenant::Membership, StatusCode>
{
    if uuid::Uuid::parse_str(id).is_err() { return Err(StatusCode::BAD_REQUEST); }
    let _db_permit = acquire_db(state).await?;
    let pool = state.db.clone();
    let user_id = user_id.to_owned();
    let id = id.to_owned();
    let membership = tokio::task::spawn_blocking(move || {
        let db = pool.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        super::tenant::membership(&db, &user_id, &id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
    membership.ok_or(StatusCode::FORBIDDEN)
}

async fn browser_tenants(State(state): State<AppState>, headers: HeaderMap)
    -> Result<Json<Value>, StatusCode>
{
    if state.auth_mode != AuthMode::Users { return Err(StatusCode::NOT_FOUND); }
    let (_, principal) = browser_session(&state, &headers).await?;
    if !state.tenancy_enabled {
        return Ok(Json(json!({"enabled": false, "tenants": []})));
    }
    let _db_permit = acquire_db(&state).await?;
    let pool = state.db.clone();
    let user_id = principal.user_id;
    let tenants = tokio::task::spawn_blocking(move || {
        let db = pool.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        super::tenant::memberships(&db, &user_id)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;
    Ok(Json(json!({"enabled": true, "tenants": tenants})))
}

async fn limit_api(State(state): State<AppState>, mut request: Request, next: Next) -> impl IntoResponse {
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
    if request.uri().path().starts_with("/api/") {
        let auth_headers = request.headers().clone();
        let authorization = auth_headers.get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()).map(str::to_owned);
        let operation = request.method().as_str().to_owned();
        match state.auth_mode {
            AuthMode::Legacy => {
                if !authorize_api(request.method(), authorization.as_deref(), state.api_token.as_deref().map(String::as_str), state.read_token.as_deref().map(String::as_str)) {
                    return Err(StatusCode::UNAUTHORIZED);
                }
            }
            AuthMode::Identities => check_identity(&state, authorization.as_deref(), &operation).await?,
            AuthMode::Users => {
                let principal = check_browser(&state, &auth_headers, &operation, !state.tenancy_enabled).await?;
                if state.tenancy_enabled {
                    let requested_tenant = auth_headers.get("x-rux-tenant")
                        .and_then(|header| header.to_str().ok())
                        .ok_or(StatusCode::BAD_REQUEST)?;
                    let membership = verified_tenant(&state, &principal.user_id, requested_tenant).await?;
                    if !super::user_auth::can(&membership.role, &operation) {
                        return Err(StatusCode::FORBIDDEN);
                    }
                    request.extensions_mut().insert(TenantContext { id: membership.id });
                }
                request.extensions_mut().insert(principal);
            },
        }
    }
    let start = Instant::now();
    let response = next.run(request).await;
    state.request_duration_micros.fetch_add(start.elapsed().as_micros().min(u64::MAX as u128) as u64, Ordering::Relaxed);
    drop(permit);
    Ok(response)
}

async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    let bearer = headers.get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    match state.auth_mode {
        AuthMode::Identities => check_identity(&state, bearer, "METRICS").await?,
        AuthMode::Users if state.tenancy_enabled => return Err(StatusCode::FORBIDDEN),
        AuthMode::Users => { check_browser(&state, &headers, "METRICS", true).await?; },
        AuthMode::Legacy => {
            let authorized = state.api_token.as_deref()
                .is_some_and(|expected| bearer.is_some_and(|value| token_matches(value, expected)));
            if !authorized { return Err(StatusCode::UNAUTHORIZED); }
        }
    }
    Ok(Json(json!({
        "requests_total": state.requests_total.load(Ordering::Relaxed),
        "rejected_total": state.rejected_total.load(Ordering::Relaxed),
        "request_duration_micros_total": state.request_duration_micros.load(Ordering::Relaxed),
        "api_slots_available": state.api_slots.available_permits(),
        "queued_slots_available": state.waiting_slots.available_permits()
    })))
}


fn content_security_policy(html: &str) -> Result<String, String> {
    use base64::Engine;
    use sha2::Digest;
    let start = html.find("<script>").ok_or("Generated UI is missing its script")? + "<script>".len();
    let end = html[start..].find("</script>").ok_or("Generated UI has an unterminated script")? + start;
    if html[end + "</script>".len()..].contains("<script>") {
        return Err("Generated UI must use a single audited inline script".into());
    }
    let hash = sha2::Sha256::digest(html[start..end].as_bytes());
    let script_hash = base64::engine::general_purpose::STANDARD.encode(hash);
    Ok(format!(
        "default-src 'none'; script-src 'sha256-{script_hash}'; style-src 'unsafe-inline'; \
         connect-src 'self'; img-src 'self' data:; manifest-src 'self'; \
         frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'"
    ))
}

async fn security_headers(State(state): State<AppState>, request: Request, next: Next) -> axum::response::Response {
    use axum::http::header::{HeaderValue, CACHE_CONTROL};
    let is_api = request.uri().path().starts_with("/api/") || request.uri().path().starts_with("/auth/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("permissions-policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=()"));
    headers.insert("content-security-policy", HeaderValue::from_str(&state.csp_header).expect("validated CSP"));
    if is_api { headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store")); }
    response
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


#[derive(Clone, Debug)]
struct RecordAccess {
    /// 0 = old/local API rows; 1 = personal rows; 2 = local administrator; 3 = active tenant.
    scope: i64,
    owner_id: Option<String>,
    tenant_id: Option<String>,
}

fn record_access(auth_mode: AuthMode, principal: Option<Extension<super::user_auth::Session>>,
    tenant: Option<Extension<TenantContext>>, tenancy_enabled: bool)
    -> Result<RecordAccess, StatusCode>
{
    match auth_mode {
        AuthMode::Legacy | AuthMode::Identities => Ok(RecordAccess {
            scope: 0, owner_id: None, tenant_id: None,
        }),
        AuthMode::Users => {
            let user = principal.ok_or(StatusCode::UNAUTHORIZED)?.0;
            if tenancy_enabled {
                let tenant_id = tenant.ok_or(StatusCode::FORBIDDEN)?.0.id;
                return Ok(RecordAccess {
                    scope: 3, owner_id: Some(user.user_id), tenant_id: Some(tenant_id),
                });
            }
            let scope = if user.role == "admin" { 2 } else { 1 };
            Ok(RecordAccess {
                scope, owner_id: Some(user.user_id), tenant_id: None,
            })
        }
    }
}

const LIST_SQL: &str = "SELECT rowid, id, payload FROM records
             WHERE rowid < ?1
               AND ((?5 = 2 AND tenant_id IS NULL)
                   OR (?5 = 0 AND tenant_id IS NULL AND owner_user_id IS NULL)
                   OR (?5 = 1 AND tenant_id IS NULL AND owner_user_id = ?6)
                   OR (?5 = 3 AND tenant_id = ?7))
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

async fn list(State(state): State<AppState>, Query(page): Query<Page>,
    principal: Option<Extension<super::user_auth::Session>>,
    tenant: Option<Extension<TenantContext>>) -> Result<Json<Value>, StatusCode> {
    let access = record_access(state.auth_mode, principal, tenant, state.tenancy_enabled)?;
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
        let rows = stmt.query_map(params![before, limit, query, field, access.scope, access.owner_id, access.tenant_id], |row| {
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

async fn create(State(state): State<AppState>, principal: Option<Extension<super::user_auth::Session>>,
    tenant: Option<Extension<TenantContext>>,
    Json(input): Json<Value>) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let access = record_access(state.auth_mode, principal, tenant, state.tenancy_enabled)?;
    let mut payload = Value::Object(validate(&input, &state.fields)?);
    let _db_permit = acquire_db(&state).await?;
    tokio::task::spawn_blocking(move || {
        let id = uuid::Uuid::new_v4().to_string();
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        db.execute("INSERT INTO records (id, payload, owner_user_id, tenant_id) VALUES (?1, ?2, ?3, ?4)",
            params![id, payload.to_string(), access.owner_id, access.tenant_id])
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        payload["id"] = json!(id);
        Ok((StatusCode::CREATED, Json(payload)))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

async fn update(State(state): State<AppState>, RoutePath(id): RoutePath<String>,
    principal: Option<Extension<super::user_auth::Session>>,
    tenant: Option<Extension<TenantContext>>,
    Json(input): Json<Value>) -> Result<Json<Value>, StatusCode> {
    let access = record_access(state.auth_mode, principal, tenant, state.tenancy_enabled)?;
    let mut payload = Value::Object(validate(&input, &state.fields)?);
    let _db_permit = acquire_db(&state).await?;
    tokio::task::spawn_blocking(move || {
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let count = db.execute("UPDATE records SET payload = ?1 WHERE id = ?2
                AND ((?3 = 2 AND tenant_id IS NULL)
                    OR (?3 = 0 AND tenant_id IS NULL AND owner_user_id IS NULL)
                    OR (?3 = 1 AND tenant_id IS NULL AND owner_user_id = ?4)
                    OR (?3 = 3 AND tenant_id = ?5))",
                params![payload.to_string(), id, access.scope, access.owner_id, access.tenant_id])
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if count == 0 { return Err(StatusCode::NOT_FOUND); }
        payload["id"] = json!(id);
        Ok(Json(payload))
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
}

async fn delete(State(state): State<AppState>, RoutePath(id): RoutePath<String>,
    principal: Option<Extension<super::user_auth::Session>>,
    tenant: Option<Extension<TenantContext>>) -> Result<impl IntoResponse, StatusCode> {
    let access = record_access(state.auth_mode, principal, tenant, state.tenancy_enabled)?;
    let _db_permit = acquire_db(&state).await?;
    tokio::task::spawn_blocking(move || {
        let db = state.db.get().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let count = db.execute("DELETE FROM records WHERE id = ?1
                AND ((?2 = 2 AND tenant_id IS NULL)
                    OR (?2 = 0 AND tenant_id IS NULL AND owner_user_id IS NULL)
                    OR (?2 = 1 AND tenant_id IS NULL AND owner_user_id = ?3)
                    OR (?2 = 3 AND tenant_id = ?4))",
                params![id, access.scope, access.owner_id, access.tenant_id])
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
    let csp_header = content_security_policy(&html)?;
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
    {
        let connection = db.get().map_err(|e| e.to_string())?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS records (
            id TEXT PRIMARY KEY, payload TEXT NOT NULL, owner_user_id TEXT, tenant_id TEXT);")
            .map_err(|e| e.to_string())?;
        let has_owner_column: i64 = connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('records') WHERE name = 'owner_user_id'",
            [], |row| row.get(0)
        ).map_err(|e| e.to_string())?;
        if has_owner_column == 0 {
            connection.execute_batch("ALTER TABLE records ADD COLUMN owner_user_id TEXT")
                .map_err(|e| format!("Cannot migrate records ownership column: {e}"))?;
        }
        let has_tenant_column: i64 = connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('records') WHERE name = 'tenant_id'",
            [], |row| row.get(0)
        ).map_err(|e| e.to_string())?;
        if has_tenant_column == 0 {
            connection.execute_batch("ALTER TABLE records ADD COLUMN tenant_id TEXT")
                .map_err(|e| format!("Cannot migrate tenant_id ownership column: {e}"))?;
        }
        connection.execute_batch("CREATE INDEX IF NOT EXISTS records_tenant_idx ON records (tenant_id)")
            .map_err(|e| e.to_string())?;
        connection.execute_batch("CREATE INDEX IF NOT EXISTS records_owner_idx ON records (owner_user_id)")
            .map_err(|e| e.to_string())?;
        super::identity::initialize(&connection).map_err(|e| e.to_string())?;
        super::user_auth::initialize(&connection).map_err(|e| e.to_string())?;
        super::tenant::initialize(&connection).map_err(|e| e.to_string())?;
    }
    let rate_limit_per_minute: u32 = std::env::var("RUX_RATE_LIMIT_PER_MINUTE")
        .unwrap_or_else(|_| "120".into())
        .parse().map_err(|_| "RUX_RATE_LIMIT_PER_MINUTE must be an integer".to_string())?;
    if !(1..=100_000).contains(&rate_limit_per_minute) {
        return Err("RUX_RATE_LIMIT_PER_MINUTE must be between 1 and 100000".into());
    }
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
    let allowed_hosts = allowed_host_names(address.ip(), std::env::var("RUX_ALLOWED_HOSTS").ok().as_deref())?;
    let auth_mode = AuthMode::parse(&std::env::var("RUX_AUTH_MODE").unwrap_or_else(|_| "legacy".into()))?;
    let tenancy_enabled = match std::env::var("RUX_TENANCY").as_deref() {
        Ok("enabled") => true,
        Ok("disabled") | Err(std::env::VarError::NotPresent) => false,
        _ => return Err("RUX_TENANCY must be enabled or disabled".into()),
    };
    if tenancy_enabled && auth_mode != AuthMode::Users {
        return Err("RUX_TENANCY=enabled requires RUX_AUTH_MODE=users. Tenant data cannot be accessed using shared API credentials.".into());
    }

    let api_token = std::env::var("RUX_API_TOKEN").ok().filter(|value| !value.is_empty());
    if auth_mode == AuthMode::Legacy && !address.ip().is_loopback()
        && api_token.as_ref().is_none_or(|token| token.len() < 32) {
        return Err("Remote legacy CRUD requires RUX_API_TOKEN with at least 32 characters; consider RUX_AUTH_MODE=identities".into());
    }
    if api_token.as_ref().is_some_and(|token| token.len() < 32) {
        return Err("RUX_API_TOKEN must contain at least 32 characters".into());
    }
    let environment = std::env::var("RUX_ENV").unwrap_or_else(|_| "development".into());
    validate_runtime_environment(&environment, auth_mode, api_token.as_deref())?;
    if std::env::var("RUX_EXPOSE_METRICS").as_deref() == Ok("true")
        && auth_mode == AuthMode::Legacy && api_token.is_none() {
        return Err("RUX_EXPOSE_METRICS=true requires RUX_API_TOKEN in legacy mode".into());
    }
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
    if auth_mode == AuthMode::Identities {
        if api_token.is_some() || read_token.is_some() {
            return Err("RUX_AUTH_MODE=identities does not use RUX_API_TOKEN or RUX_API_READ_TOKEN; remove these shared credentials".into());
        }
        let connection = db.get().map_err(|e| e.to_string())?;
        if super::identity::active_count(&connection).map_err(|e| e.to_string())? == 0 {
            return Err("RUX_AUTH_MODE=identities requires at least one active identity. Run: rux identity create <app-directory> <name> admin".into());
        }
    }
    if auth_mode == AuthMode::Users && !address.ip().is_loopback()
        && std::env::var("RUX_TLS_TERMINATED_AT_PROXY").as_deref() != Ok("true") {
        return Err("Remote user login requires HTTPS at a trusted reverse proxy. Configure TLS and network isolation, then explicitly set RUX_TLS_TERMINATED_AT_PROXY=true".into());
    }
    if auth_mode == AuthMode::Users {
        if api_token.is_some() || read_token.is_some() {
            return Err("RUX_AUTH_MODE=users does not use shared API tokens; remove RUX_API_TOKEN and RUX_API_READ_TOKEN".into());
        }
        let connection = db.get().map_err(|e| e.to_string())?;
        if super::user_auth::enabled_count(&connection).map_err(|e| e.to_string())? == 0 {
            return Err("RUX_AUTH_MODE=users requires an active login user. Run: rux user create <app-directory> <name> admin".into());
        }
    }
    if tenancy_enabled {
        let connection = db.get().map_err(|e| e.to_string())?;
        if super::tenant::active_count(&connection).map_err(|e| e.to_string())? == 0 {
            return Err("RUX_TENANCY=enabled requires an active tenant membership. Use: rux tenant create / grant".into());
        }
    }
    let secure_cookie = environment == "production" || !address.ip().is_loopback();
    let accepting = Arc::new(AtomicBool::new(true));
    let state = AppState { db, fields: Arc::new(fields), html: Arc::new(html), manifest: Arc::new(manifest_content), csp_header: Arc::new(csp_header), api_token: api_token.map(Arc::new), read_token: read_token.map(Arc::new), auth_mode, tenancy_enabled, secure_cookie, allowed_hosts: Arc::new(allowed_hosts), rate_windows: Arc::new(Mutex::new(HashMap::new())), rate_limit_per_minute, accepting: Arc::clone(&accepting), api_slots: Arc::new(Semaphore::new(max_requests)), waiting_slots: Arc::new(Semaphore::new(max_waiting)), api_wait: Duration::from_millis(wait_ms), db_slots: Arc::new(Semaphore::new(pool_size as usize)), db_wait: Duration::from_millis(db_wait_ms), requests_total: Arc::new(AtomicU64::new(0)), rejected_total: Arc::new(AtomicU64::new(0)), request_duration_micros: Arc::new(AtomicU64::new(0)) };
    let state_for_rate = state.clone();
    let state_for_host = state.clone();
    let state_for_security = state.clone();
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
    let router = if auth_mode == AuthMode::Users {
        router
            .route("/auth/login", post(browser_login).with_state(state.clone()))
            .route("/auth/session", get(browser_me).with_state(state.clone()))
            .route("/auth/logout", post(browser_logout).with_state(state.clone()))
            .route("/auth/tenants", get(browser_tenants).with_state(state.clone()))
    } else {
        router
    };
    let router = if std::env::var("RUX_EXPOSE_METRICS").as_deref() == Ok("true") {
        router.route("/metrics", get(metrics).with_state(state))
    } else {
        router
    };
    let router = router.layer(DefaultBodyLimit::max(32 * 1024))
        .layer(middleware::from_fn_with_state(state_for_rate, rate_gate))
        .layer(middleware::from_fn_with_state(state_for_host, host_origin_guard))
        .layer(middleware::from_fn_with_state(state_for_security, security_headers));
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|e| e.to_string())?;
    println!("RUX native CRUD: http://{address} (SQLite)");
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
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

fn validate_runtime_environment(environment: &str, mode: AuthMode, api_token: Option<&str>) -> Result<(), String> {
    match environment {
        "development" | "test" => Ok(()),
        "production" if mode == AuthMode::Identities || mode == AuthMode::Users => Ok(()),
        "production" => {
            if api_token.is_none_or(|token| token.len() < 32) {
                Err("Production legacy mode requires RUX_API_TOKEN with at least 32 characters. Prefer RUX_AUTH_MODE=identities and HTTPS at ingress.".into())
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
    fn browser_origin_must_match_request_host() {
        assert!(same_origin("http://localhost:3000", "localhost:3000"));
        assert!(same_origin("https://api.example.com", "api.example.com"));
        assert!(!same_origin("https://attacker.example", "api.example.com"));
        assert!(!same_origin("http://localhost:8080", "localhost:3000"));
        assert!(!same_origin("null", "localhost:3000"));
    }
    #[test]
    fn remote_servers_require_explicit_host_allowlist() {
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let public_ip: IpAddr = "0.0.0.0".parse().unwrap();
        assert!(allowed_host_names(loopback, None).unwrap().contains(&"localhost".into()));
        assert!(allowed_host_names(public_ip, None).is_err());
        let names = allowed_host_names(public_ip, Some("api.example.com, 192.0.2.10")).unwrap();
        assert_eq!(names, vec!["api.example.com", "192.0.2.10"]);
        assert!(allowed_host_names(public_ip, Some("*,api.example.com")).is_err());
        assert!(allowed_host_names(public_ip, Some("api.example.com:3000")).is_err());
        assert_eq!(normalized_host("Example.Com:3000").as_deref(), Some("example.com"));
    }
    #[test]
    fn rate_limit_is_per_client_and_resets_after_one_minute() {
        let windows = Mutex::new(HashMap::new());
        let first: IpAddr = "127.0.0.1".parse().unwrap();
        let second: IpAddr = "127.0.0.2".parse().unwrap();
        let now = Instant::now();
        assert!(within_rate_limit(&windows, first, 2, now));
        assert!(within_rate_limit(&windows, first, 2, now));
        assert!(!within_rate_limit(&windows, first, 2, now));
        assert!(within_rate_limit(&windows, second, 2, now));
        assert!(within_rate_limit(&windows, first, 2, now + Duration::from_secs(60)));
    }
    #[test]
    fn csp_allows_only_the_exact_generated_inline_script() {
        let html = "<html><script>console.log('hello');</script></html>";
        let policy = content_security_policy(html).unwrap();
        assert!(policy.contains("default-src 'none'"));
        assert!(policy.contains("script-src 'sha256-"));
        assert!(!policy.contains("script-src 'unsafe-inline'"));
        assert_ne!(policy, content_security_policy("<html><script>console.log('other');</script></html>").unwrap());
        assert!(content_security_policy("<html></html>").is_err());
    }
    #[test]
    fn tenant_mode_requires_user_login() {
        assert_eq!(AuthMode::parse("users").unwrap(), AuthMode::Users);
        assert!(AuthMode::parse("tenant").is_err());
    }
    #[test]
    fn browser_user_mode_requires_explicit_selection() {
        assert_eq!(AuthMode::parse("users").unwrap(), AuthMode::Users);
        assert!(validate_runtime_environment("production", AuthMode::Users, None).is_ok());
        assert_eq!(session_cookie(&HeaderMap::new()), None);
        let mut headers = HeaderMap::new();
        let value = format!("rux_session=ruxs_{}", "a".repeat(64));
        headers.insert(axum::http::header::COOKIE, value.parse().unwrap());
        assert_eq!(session_cookie(&headers), Some(format!("ruxs_{}", "a".repeat(64))));
        headers.append(axum::http::header::COOKIE, value.parse().unwrap());
        assert_eq!(session_cookie(&headers), None);
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::COOKIE, "foo=bar; rux_session=ruxs_123".parse().unwrap());
        assert!(session_cookie(&headers).is_none());
    }
    #[test]
    fn identity_mode_is_explicit_and_production_compatible() {
        assert_eq!(AuthMode::parse("identities").unwrap(), AuthMode::Identities);
        assert!(AuthMode::parse("anonymous").is_err());
        assert!(validate_runtime_environment("production", AuthMode::Identities, None).is_ok());
    }
    #[test]
    fn production_requires_api_credentials() {
        let strong = "a".repeat(32);
        assert!(validate_runtime_environment("development", AuthMode::Legacy, None).is_ok());
        assert!(validate_runtime_environment("test", AuthMode::Legacy, None).is_ok());
        assert!(validate_runtime_environment("production", AuthMode::Legacy, None).is_err());
        assert!(validate_runtime_environment("production", AuthMode::Legacy, Some("short")).is_err());
        assert!(validate_runtime_environment("production", AuthMode::Legacy, Some(&strong)).is_ok());
        assert!(validate_runtime_environment("prod", AuthMode::Legacy, Some(&strong)).is_err());
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
            csp_header: Arc::new(String::new()),
            api_token: None,
            read_token: None,
            auth_mode: AuthMode::Legacy,
            tenancy_enabled: false,
            secure_cookie: false,
            allowed_hosts: Arc::new(vec!["localhost".into()]),
            rate_windows: Arc::new(Mutex::new(HashMap::new())),
            rate_limit_per_minute: 120,
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
    fn record_ownership_isolation_denies_cross_user_access() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE records (id TEXT PRIMARY KEY, payload TEXT NOT NULL, owner_user_id TEXT);").unwrap();
        for (id, owner) in [("legacy", None), ("alice", Some("user-a")), ("bob", Some("user-b"))] {
            db.execute("INSERT INTO records (id,payload,owner_user_id) VALUES (?1,?2,?3)",
                params![id, format!("{{\"name\":\"{id}\"}}"), owner]).unwrap();
        }
        let visible = |scope: i64, owner: Option<&str>| -> Vec<String> {
            let mut stmt = db.prepare(LIST_SQL).unwrap();
            let rows = stmt.query_map(params![i64::MAX, 50, "", "", scope, owner],
                |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert_eq!(visible(0, None), vec!["legacy".to_owned()]);
        assert_eq!(visible(1, Some("user-a")), vec!["alice".to_owned()]);
        assert_eq!(visible(1, Some("user-b")), vec!["bob".to_owned()]);
        assert_eq!(visible(1, Some("missing")), Vec::<String>::new());
        assert_eq!(visible(2, Some("user-a")).len(), 3);

        let alice = crate::user_auth::Session {
            user_id: "user-a".into(), username: "alice".into(), role: "editor".into(),
            expires_at: 1000, csrf: "test".into(),
        };
        let editor = record_access(AuthMode::Users, Some(Extension(alice))).unwrap();
        assert_eq!(editor.scope, 1);
        assert_eq!(editor.owner_id.as_deref(), Some("user-a"));
        assert!(matches!(record_access(AuthMode::Users, None), Err(StatusCode::UNAUTHORIZED)));

        assert_eq!(db.execute(
            "UPDATE records SET payload = ?1 WHERE id = ?2 AND
             (?3 = 2 OR (?3 = 0 AND owner_user_id IS NULL) OR (?3 = 1 AND owner_user_id = ?4))",
            params!["{}", "bob", editor.scope, editor.owner_id]
        ).unwrap(), 0);
        assert_eq!(db.execute(
            "DELETE FROM records WHERE id = ?1 AND
             (?2 = 2 OR (?2 = 0 AND owner_user_id IS NULL) OR (?2 = 1 AND owner_user_id = ?3))",
            params!["bob", editor.scope, editor.owner_id]
        ).unwrap(), 0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM records", [], |row| row.get::<_, i64>(0)).unwrap(), 3);
    }

    #[test]
    fn search_uses_bound_parameters_and_resists_sql_injection() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE records (id TEXT PRIMARY KEY, payload TEXT NOT NULL, owner_user_id TEXT);").unwrap();
        db.execute("INSERT INTO records (id, payload) VALUES (?1, ?2)",
            params!["one", r#"{"nome":"Alice","email":"alice@example.com"}"#]).unwrap();
        db.execute("INSERT INTO records (id, payload) VALUES (?1, ?2)",
            params!["two", r#"{"nome":"Bob","email":"bob@example.com"}"#]).unwrap();
        let count = |q: &str, field: &str| -> usize {
            let mut statement = db.prepare(LIST_SQL).unwrap();
            let rows = statement.query_map(params![i64::MAX, 50, q, field, 0, Option::<String>::None], |row| row.get::<_, String>(1))
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
