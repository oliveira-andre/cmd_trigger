//! The CmdTrigger server: your own relay between your phone (or any browser, script, iPhone
//! Shortcut or AI assistant) and your computers' agents. Accounts are email + password; each
//! computer connects with its own agent token; scripts and AI assistants use API tokens.
//! See `cmdtrigger_protocol` for the endpoints and messages. Data lives in one SQLite file.

mod dashboard;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use cmdtrigger_protocol::{
    AgentMessage, ApiError, CatalogEntry, ComputerView, CreatedToken, Credentials, NewToken, RegisterComputer,
    Registered, RunRequest, RunStatus, RunView, ServerMessage, Session, TargetKind, TokenView, mcp,
};
use futures_util::{SinkExt, StreamExt};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Who may create an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signup {
    /// Only while the server has no accounts (the default: the first account is yours).
    First,
    Open,
    Closed,
}

impl Signup {
    pub fn from_env(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_lowercase()) {
            Some(v) if matches!(v.as_str(), "true" | "1" | "yes" | "on" | "open") => Signup::Open,
            Some(v) if matches!(v.as_str(), "false" | "0" | "no" | "off" | "closed") => Signup::Closed,
            _ => Signup::First,
        }
    }
}

/// A session unused for this long must sign in again.
const SESSION_IDLE: u64 = 60 * 24 * 3600;
const MAX_WAIT: u64 = 60;
const KEEP_RUNS: i64 = 1000;
/// Failed sign-ins in a row before an account is locked for a while.
const MAX_FAILURES: u32 = 5;

type AgentKey = (String, String); // (computer id, ground)

pub struct Server {
    db: Mutex<Connection>,
    signup: Signup,
    agents: Mutex<HashMap<AgentKey, (u64, mpsc::UnboundedSender<ServerMessage>)>>,
    runs: Mutex<HashMap<String, watch::Sender<Option<RunView>>>>,
    failures: Mutex<HashMap<String, (u32, u64)>>,
    next_conn: Mutex<u64>,
    dummy_hash: String,
}

#[derive(Debug)]
pub struct HttpError(StatusCode, String, &'static str);

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.0, Json(ApiError { error: self.1, code: self.2.into() })).into_response()
    }
}

impl From<rusqlite::Error> for HttpError {
    fn from(e: rusqlite::Error) -> Self {
        tracing::error!("database: {e}");
        HttpError(StatusCode::INTERNAL_SERVER_ERROR, "database error".into(), "internal")
    }
}

type ApiResult<T> = Result<T, HttpError>;

fn bad(message: &str) -> HttpError {
    HttpError(StatusCode::BAD_REQUEST, message.into(), "invalid")
}
fn unauthorized() -> HttpError {
    HttpError(StatusCode::UNAUTHORIZED, "sign in first".into(), "unauthorized")
}
fn not_found(message: &str) -> HttpError {
    HttpError(StatusCode::NOT_FOUND, message.into(), "not_found")
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("the OS has randomness");
    hex::encode(b)
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn normalize_email(email: &str) -> Option<String> {
    let email = email.trim().to_lowercase();
    let (user, domain) = email.split_once('@')?;
    (!user.is_empty() && domain.contains('.') && !domain.starts_with('.') && !email.contains(char::is_whitespace))
        .then_some(email)
}

fn hash_password(password: &str) -> String {
    let salt = SaltString::encode_b64(&{
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).expect("randomness");
        b
    })
    .expect("16 bytes is a valid salt");
    Argon2::default().hash_password(password.as_bytes(), &salt).expect("argon2 hashes any password").to_string()
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
}

#[derive(Clone, Copy, PartialEq)]
enum Auth {
    Session,
    /// An API token: runs and reads, no account management.
    Token,
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::trim)
}

impl Server {
    pub fn open(path: &FsPath, signup: Signup) -> rusqlite::Result<Arc<Self>> {
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS users (
                 id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS tokens (
                 id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE,
                 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 kind TEXT NOT NULL, name TEXT NOT NULL, created_at INTEGER NOT NULL, last_used_at INTEGER);
             CREATE TABLE IF NOT EXISTS computers (
                 id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 name TEXT NOT NULL, os TEXT NOT NULL DEFAULT '', agent_token_hash TEXT NOT NULL UNIQUE,
                 catalog TEXT NOT NULL DEFAULT '{}', created_at INTEGER NOT NULL, last_seen_at INTEGER);
             CREATE TABLE IF NOT EXISTS runs (
                 id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                 computer_id TEXT NOT NULL, computer_name TEXT NOT NULL, kind TEXT NOT NULL, target TEXT NOT NULL,
                 target_name TEXT NOT NULL, params TEXT, status TEXT NOT NULL, exit_code INTEGER, output TEXT NOT NULL DEFAULT '',
                 source TEXT NOT NULL, created_at INTEGER NOT NULL, finished_at INTEGER);
             CREATE INDEX IF NOT EXISTS runs_by_user ON runs (user_id, created_at);",
        )?;
        db.execute(
            "DELETE FROM tokens WHERE kind = 'session' AND COALESCE(last_used_at, created_at) < ?1",
            params![now().saturating_sub(SESSION_IDLE)],
        )?;
        Ok(Arc::new(Self {
            db: Mutex::new(db),
            signup,
            agents: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
            next_conn: Mutex::new(0),
            dummy_hash: hash_password("not a real password"),
        }))
    }

    /// The user behind a session or API token.
    fn user(&self, headers: &HeaderMap, allow: &[Auth]) -> ApiResult<String> {
        let token = bearer(headers).ok_or_else(unauthorized)?;
        let db = self.db.lock().unwrap();
        let row: Option<(String, String, String)> = db
            .query_row("SELECT id, user_id, kind FROM tokens WHERE token_hash = ?1", params![token_hash(token)], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let (id, user, kind) = row.ok_or_else(unauthorized)?;
        let auth = if kind == "session" { Auth::Session } else { Auth::Token };
        if !allow.contains(&auth) {
            return Err(HttpError(
                StatusCode::FORBIDDEN,
                "an API token can't do that; use the dashboard".into(),
                "forbidden",
            ));
        }
        db.execute("UPDATE tokens SET last_used_at = ?1 WHERE id = ?2", params![now(), id])?;
        Ok(user)
    }

    fn new_token(&self, user: &str, kind: &str, name: &str) -> ApiResult<(String, String)> {
        let token = format!("{}_{}", if kind == "session" { "cts" } else { "cta" }, random_hex(24));
        let id = random_hex(8);
        self.db.lock().unwrap().execute(
            "INSERT INTO tokens (id, token_hash, user_id, kind, name, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, token_hash(&token), user, kind, name, now()],
        )?;
        Ok((id, token))
    }

    pub fn list_users(&self) -> rusqlite::Result<Vec<(String, u64, u64)>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT email, created_at, (SELECT COUNT(*) FROM computers c WHERE c.user_id = u.id) FROM users u ORDER BY created_at")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect()
    }

    pub fn delete_user(&self, email: &str) -> rusqlite::Result<bool> {
        let email = normalize_email(email).unwrap_or_default();
        Ok(self.db.lock().unwrap().execute("DELETE FROM users WHERE email = ?1", params![email])? > 0)
    }

    /// Set a new password (for an owner who forgot theirs); signs out every session.
    pub fn reset_password(&self, email: &str, password: &str) -> rusqlite::Result<bool> {
        let email = normalize_email(email).unwrap_or_default();
        let db = self.db.lock().unwrap();
        let changed = db
            .execute("UPDATE users SET password_hash = ?1 WHERE email = ?2", params![hash_password(password), email])?;
        db.execute(
            "DELETE FROM tokens WHERE kind = 'session' AND user_id = (SELECT id FROM users WHERE email = ?1)",
            params![email],
        )?;
        Ok(changed > 0)
    }

    fn online(&self, computer: &str) -> Vec<String> {
        let agents = self.agents.lock().unwrap();
        let mut grounds: Vec<String> = agents.keys().filter(|(c, _)| c == computer).map(|(_, g)| g.clone()).collect();
        grounds.sort();
        grounds
    }

    fn computers(&self, user: &str) -> ApiResult<Vec<ComputerView>> {
        let rows: Vec<(String, String, String, String, Option<u64>)> = {
            let db = self.db.lock().unwrap();
            let mut stmt = db.prepare("SELECT id, name, os, catalog, last_seen_at FROM computers WHERE user_id = ?1 ORDER BY name COLLATE NOCASE")?;
            stmt.query_map(params![user], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<Result<_, _>>()?
        };
        Ok(rows
            .into_iter()
            .map(|(id, name, os, catalog, last_seen)| {
                let online = self.online(&id);
                ComputerView { catalog: flatten_catalog(&catalog), online, id, name, os, last_seen }
            })
            .collect())
    }

    fn run_row(&self, id: &str) -> ApiResult<Option<RunView>> {
        let db = self.db.lock().unwrap();
        Ok(db
            .query_row(
                "SELECT id, computer_id, computer_name, kind, target, target_name, params, status, exit_code, output, source, created_at, finished_at
                 FROM runs WHERE id = ?1",
                params![id],
                run_from_row,
            )
            .optional()?)
    }

    /// Queue a run on a computer, send it to its agent, and optionally wait for the result.
    async fn start_run(&self, user: &str, request: RunRequest, source: &str) -> ApiResult<RunView> {
        let computers = self.computers(user)?;
        let wanted = request.computer.trim();
        let computer = computers
            .iter()
            .find(|c| c.id == wanted)
            .or_else(|| computers.iter().find(|c| c.name.eq_ignore_ascii_case(wanted)))
            .or_else(|| if wanted.is_empty() && computers.len() == 1 { computers.first() } else { None })
            .ok_or_else(|| not_found(&format!("no computer called \"{wanted}\"")))?;
        let entry = resolve_target(&computer.catalog, &request.target, request.kind)
            .ok_or_else(|| not_found(&format!("\"{}\" has nothing called \"{}\"", computer.name, request.target)))?;
        let sender =
            self.agents.lock().unwrap().get(&(computer.id.clone(), entry.ground.clone())).map(|(_, s)| s.clone());
        let id = random_hex(8);
        let params = request.params.filter(|p| !p.trim().is_empty() && entry.allow_params);
        let status = if sender.is_some() { RunStatus::Queued } else { RunStatus::Offline };
        let kind = match entry.kind {
            TargetKind::Command => "command",
            TargetKind::Automation => "automation",
        };
        {
            let db = self.db.lock().unwrap();
            db.execute(
                "INSERT INTO runs (id, user_id, computer_id, computer_name, kind, target, target_name, params, status, output, source, created_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    id,
                    user,
                    computer.id,
                    computer.name,
                    kind,
                    entry.id,
                    entry.name,
                    params,
                    status_text(status),
                    if sender.is_none() { format!("{} is offline.", computer.name) } else { String::new() },
                    source,
                    now(),
                    if sender.is_none() { Some(now()) } else { None }
                ],
            )?;
            db.execute(
                "DELETE FROM runs WHERE user_id = ?1 AND id NOT IN (SELECT id FROM runs WHERE user_id = ?1 ORDER BY created_at DESC LIMIT ?2)",
                params![user, KEEP_RUNS],
            )?;
        }
        let Some(sender) = sender else { return self.run_row(&id)?.ok_or_else(|| not_found("run")) };
        let (tx, mut rx) = watch::channel(None);
        self.runs.lock().unwrap().insert(id.clone(), tx);
        let _ = sender.send(ServerMessage::Run {
            run_id: id.clone(),
            kind: entry.kind,
            id: entry.id.clone(),
            params,
            source: source.into(),
        });
        let wait = request.wait.unwrap_or(0).min(MAX_WAIT);
        if wait > 0 {
            let finished = async {
                loop {
                    if rx.borrow().as_ref().is_some_and(|r| r.status.finished()) {
                        return;
                    }
                    if rx.changed().await.is_err() {
                        return;
                    }
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(wait), finished).await;
        }
        self.run_row(&id)?.ok_or_else(|| not_found("run"))
    }

    fn record_result(&self, computer: &str, run_id: &str, status: RunStatus, exit_code: Option<i32>, output: &str) {
        let finished = status.finished().then(now);
        let updated = self.db.lock().unwrap().execute(
            "UPDATE runs SET status = ?1, exit_code = ?2, output = ?3, finished_at = ?4 WHERE id = ?5 AND computer_id = ?6",
            params![status_text(status), exit_code, output, finished, run_id, computer],
        );
        if !matches!(updated, Ok(1)) {
            return;
        }
        let view = self.run_row(run_id).ok().flatten();
        let mut runs = self.runs.lock().unwrap();
        if let Some(tx) = runs.get(run_id) {
            tx.send_replace(view);
        }
        if status.finished() {
            runs.remove(run_id);
        }
    }
}

fn status_text(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Queued => "queued",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
        RunStatus::Offline => "offline",
        RunStatus::TimedOut => "timed_out",
    }
}

fn parse_status(text: &str) -> RunStatus {
    serde_json::from_value(Value::String(text.into())).unwrap_or(RunStatus::Failed)
}

fn run_from_row(r: &rusqlite::Row) -> rusqlite::Result<RunView> {
    let kind: String = r.get(3)?;
    let status: String = r.get(7)?;
    Ok(RunView {
        id: r.get(0)?,
        computer_id: r.get(1)?,
        computer_name: r.get(2)?,
        kind: if kind == "automation" { TargetKind::Automation } else { TargetKind::Command },
        target: r.get(4)?,
        target_name: r.get(5)?,
        params: r.get(6)?,
        status: parse_status(&status),
        exit_code: r.get(8)?,
        output: r.get(9)?,
        source: r.get(10)?,
        created_at: r.get(11)?,
        finished_at: r.get(12)?,
    })
}

/// The stored catalog is `{"foreground": [...], "background": [...]}`.
fn flatten_catalog(text: &str) -> Vec<CatalogEntry> {
    let map: HashMap<String, Vec<CatalogEntry>> = serde_json::from_str(text).unwrap_or_default();
    let mut entries: Vec<CatalogEntry> = Vec::new();
    for ground in ["foreground", "background"] {
        entries.extend(map.get(ground).cloned().unwrap_or_default());
    }
    entries
}

fn resolve_target(catalog: &[CatalogEntry], target: &str, kind: Option<TargetKind>) -> Option<CatalogEntry> {
    let target = target.trim();
    let matches = |e: &&CatalogEntry| kind.is_none_or(|k| e.kind == k);
    catalog
        .iter()
        .filter(matches)
        .find(|e| e.kind == TargetKind::Command && e.name.eq_ignore_ascii_case(target))
        .or_else(|| catalog.iter().filter(matches).find(|e| e.id == target))
        .or_else(|| catalog.iter().filter(matches).find(|e| e.name.eq_ignore_ascii_case(target)))
        .cloned()
}

// ---------- HTTP ----------

pub fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/api/v1/health", get(|| async { Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })) }))
        .route("/api/v1/signup", post(signup))
        .route("/api/v1/login", post(login))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/me", get(me))
        .route("/api/v1/computers", get(list_computers))
        .route("/api/v1/computers/register", post(register_computer))
        .route("/api/v1/computers/{id}", delete(delete_computer))
        .route("/api/v1/run", post(run))
        .route("/api/v1/runs", get(list_runs))
        .route("/api/v1/runs/{id}", get(get_run))
        .route("/api/v1/tokens", get(list_tokens).post(create_token))
        .route("/api/v1/tokens/{id}", delete(delete_token))
        .route("/api/v1/agent", get(agent_socket))
        .route("/api/v1/mcp", post(mcp_endpoint).get(|| async { StatusCode::METHOD_NOT_ALLOWED }))
        .merge(dashboard::routes())
        .with_state(server)
}

async fn signup(State(server): State<Arc<Server>>, Json(c): Json<Credentials>) -> ApiResult<Json<Session>> {
    let email = normalize_email(&c.email).ok_or_else(|| bad("That doesn't look like an email address."))?;
    if c.password.chars().count() < 8 {
        return Err(bad("Use a password of at least 8 characters."));
    }
    let allowed = match server.signup {
        Signup::Open => true,
        Signup::Closed => false,
        Signup::First => {
            server.db.lock().unwrap().query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? == 0
        }
    };
    if !allowed {
        return Err(HttpError(StatusCode::FORBIDDEN, "This server doesn't take new accounts.".into(), "signup_closed"));
    }
    let hash = tokio::task::spawn_blocking(move || hash_password(&c.password)).await.expect("hashing doesn't panic");
    let id = random_hex(8);
    let inserted = server.db.lock().unwrap().execute(
        "INSERT INTO users (id, email, password_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, email, hash, now()],
    );
    match inserted {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
            return Err(HttpError(
                StatusCode::CONFLICT,
                "There is already an account with that email.".into(),
                "exists",
            ));
        }
        Err(e) => return Err(e.into()),
    }
    let (_, token) = server.new_token(&id, "session", c.device.trim())?;
    Ok(Json(Session { token, email }))
}

async fn login(State(server): State<Arc<Server>>, Json(c): Json<Credentials>) -> ApiResult<Json<Session>> {
    let email = normalize_email(&c.email).unwrap_or_default();
    let locked_until =
        server.failures.lock().unwrap().get(&email).filter(|(n, _)| *n >= MAX_FAILURES).map(|(_, until)| *until);
    if locked_until.is_some_and(|until| now() < until) {
        return Err(HttpError(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many tries. Wait a minute and try again.".into(),
            "locked",
        ));
    }
    let row: Option<(String, String)> = server
        .db
        .lock()
        .unwrap()
        .query_row("SELECT id, password_hash FROM users WHERE email = ?1", params![email], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let hash = row.as_ref().map(|r| r.1.clone()).unwrap_or_else(|| server.dummy_hash.clone());
    let password = c.password.clone();
    let ok =
        tokio::task::spawn_blocking(move || verify_password(&password, &hash)).await.expect("verify doesn't panic")
            && row.is_some();
    if !ok {
        let mut failures = server.failures.lock().unwrap();
        let entry = failures.entry(email).or_insert((0, 0));
        entry.0 += 1;
        if entry.0 >= MAX_FAILURES {
            entry.1 = now() + 60 * 2u64.pow((entry.0 - MAX_FAILURES).min(6));
        }
        return Err(HttpError(StatusCode::UNAUTHORIZED, "Wrong email or password.".into(), "bad_credentials"));
    }
    server.failures.lock().unwrap().remove(&email);
    let (user, _) = row.unwrap();
    let (_, token) = server.new_token(&user, "session", c.device.trim())?;
    Ok(Json(Session { token, email }))
}

async fn logout(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult<StatusCode> {
    if let Some(token) = bearer(&headers) {
        server
            .db
            .lock()
            .unwrap()
            .execute("DELETE FROM tokens WHERE token_hash = ?1 AND kind = 'session'", params![token_hash(token)])?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn me(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let user = server.user(&headers, &[Auth::Session, Auth::Token])?;
    let email: String =
        server.db.lock().unwrap().query_row("SELECT email FROM users WHERE id = ?1", params![user], |r| r.get(0))?;
    Ok(Json(json!({ "email": email })))
}

async fn list_computers(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult<Json<Vec<ComputerView>>> {
    let user = server.user(&headers, &[Auth::Session, Auth::Token])?;
    Ok(Json(server.computers(&user)?))
}

async fn register_computer(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(r): Json<RegisterComputer>,
) -> ApiResult<Json<Registered>> {
    let user = server.user(&headers, &[Auth::Session])?;
    let name = r.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(bad("The computer needs a name (up to 80 characters)."));
    }
    let token = format!("ctk_{}", random_hex(24));
    let db = server.db.lock().unwrap();
    // Signing in again from the same computer name replaces its token rather than adding a twin.
    let existing: Option<String> = db
        .query_row(
            "SELECT id FROM computers WHERE user_id = ?1 AND name = ?2 COLLATE NOCASE",
            params![user, name],
            |r| r.get(0),
        )
        .optional()?;
    let id = match existing {
        Some(id) => {
            db.execute(
                "UPDATE computers SET agent_token_hash = ?1, os = ?2 WHERE id = ?3",
                params![token_hash(&token), r.os, id],
            )?;
            id
        }
        None => {
            let id = random_hex(8);
            db.execute(
                "INSERT INTO computers (id, user_id, name, os, agent_token_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, user, name, r.os, token_hash(&token), now()],
            )?;
            id
        }
    };
    drop(db);
    // An agent still connected with the old token must stop.
    let stale: Vec<_> =
        server.agents.lock().unwrap().iter().filter(|((c, _), _)| *c == id).map(|(_, (_, tx))| tx.clone()).collect();
    for tx in stale {
        let _ = tx.send(ServerMessage::Rejected { message: "This computer signed in again.".into() });
    }
    Ok(Json(Registered { computer_id: id, agent_token: token }))
}

async fn delete_computer(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let user = server.user(&headers, &[Auth::Session])?;
    let deleted =
        server.db.lock().unwrap().execute("DELETE FROM computers WHERE id = ?1 AND user_id = ?2", params![id, user])?;
    if deleted == 0 {
        return Err(not_found("no such computer"));
    }
    let connected: Vec<_> =
        server.agents.lock().unwrap().iter().filter(|((c, _), _)| *c == id).map(|(_, (_, tx))| tx.clone()).collect();
    for tx in connected {
        let _ = tx.send(ServerMessage::Rejected { message: "This computer was removed from the account.".into() });
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn run(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(request): Json<RunRequest>,
) -> ApiResult<Json<RunView>> {
    let source = match bearer(&headers) {
        Some(t) if t.starts_with("cta_") => "api",
        _ => "dashboard",
    };
    let user = server.user(&headers, &[Auth::Session, Auth::Token])?;
    Ok(Json(server.start_run(&user, request, source).await?))
}

async fn list_runs(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Json<Vec<RunView>>> {
    let user = server.user(&headers, &[Auth::Session, Auth::Token])?;
    let limit: i64 = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50).clamp(1, 500);
    let db = server.db.lock().unwrap();
    let mut stmt = db.prepare(
        "SELECT id, computer_id, computer_name, kind, target, target_name, params, status, exit_code, output, source, created_at, finished_at
         FROM runs WHERE user_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
    )?;
    let runs = stmt.query_map(params![user, limit], run_from_row)?.collect::<Result<_, _>>()?;
    Ok(Json(runs))
}

async fn get_run(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Json<RunView>> {
    let user = server.user(&headers, &[Auth::Session, Auth::Token])?;
    let owner: Option<String> = server
        .db
        .lock()
        .unwrap()
        .query_row("SELECT user_id FROM runs WHERE id = ?1", params![id], |r| r.get(0))
        .optional()?;
    if owner.as_deref() != Some(user.as_str()) {
        return Err(not_found("no such run"));
    }
    Ok(Json(server.run_row(&id)?.ok_or_else(|| not_found("no such run"))?))
}

async fn list_tokens(State(server): State<Arc<Server>>, headers: HeaderMap) -> ApiResult<Json<Vec<TokenView>>> {
    let user = server.user(&headers, &[Auth::Session])?;
    let db = server.db.lock().unwrap();
    let mut stmt = db.prepare(
        "SELECT id, name, created_at, last_used_at FROM tokens WHERE user_id = ?1 AND kind = 'api' ORDER BY created_at",
    )?;
    let tokens = stmt
        .query_map(params![user], |r| {
            Ok(TokenView { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, last_used_at: r.get(3)? })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Json(tokens))
}

async fn create_token(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Json(t): Json<NewToken>,
) -> ApiResult<Json<CreatedToken>> {
    let user = server.user(&headers, &[Auth::Session])?;
    let name =
        if t.name.trim().is_empty() { "API token".to_string() } else { t.name.trim().chars().take(60).collect() };
    let (id, token) = server.new_token(&user, "api", &name)?;
    Ok(Json(CreatedToken { id, name, token }))
}

async fn delete_token(
    State(server): State<Arc<Server>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let user = server.user(&headers, &[Auth::Session])?;
    server
        .db
        .lock()
        .unwrap()
        .execute("DELETE FROM tokens WHERE id = ?1 AND user_id = ?2 AND kind = 'api'", params![id, user])?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- the agent socket ----------

async fn agent_socket(State(server): State<Arc<Server>>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let token = bearer(&headers).unwrap_or("");
    let computer: Option<(String, String)> = server
        .db
        .lock()
        .unwrap()
        .query_row("SELECT id, user_id FROM computers WHERE agent_token_hash = ?1", params![token_hash(token)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()
        .ok()
        .flatten();
    let Some((computer, _user)) = computer else {
        return HttpError(
            StatusCode::UNAUTHORIZED,
            "This computer's token is not valid. Sign in again from the app.".into(),
            "unauthorized",
        )
        .into_response();
    };
    ws.on_upgrade(move |socket| agent_session(server, computer, socket))
}

async fn agent_session(server: Arc<Server>, computer: String, socket: WebSocket) {
    let (mut tx, mut rx) = socket.split();
    let send = |m: &ServerMessage| Message::Text(serde_json::to_string(m).expect("messages serialize").into());
    let hello = tokio::time::timeout(Duration::from_secs(15), rx.next()).await;
    let Ok(Some(Ok(Message::Text(text)))) = hello else { return };
    let Ok(AgentMessage::Hello { name, os, version, ground, catalog }) = serde_json::from_str::<AgentMessage>(&text)
    else {
        return;
    };
    let ground = if ground == "background" { "background".to_string() } else { "foreground".to_string() };
    tracing::info!("Agent connected: {name} ({ground}, {os}, v{version})");
    let store_catalog = |catalog: &[CatalogEntry]| {
        let db = server.db.lock().unwrap();
        let current: String = db
            .query_row("SELECT catalog FROM computers WHERE id = ?1", params![computer], |r| r.get(0))
            .unwrap_or_default();
        let mut map: HashMap<String, Value> = serde_json::from_str(&current).unwrap_or_default();
        let entries: Vec<CatalogEntry> = catalog
            .iter()
            .cloned()
            .map(|mut e| {
                e.ground = ground.clone();
                e
            })
            .collect();
        map.insert(ground.clone(), serde_json::to_value(entries).unwrap_or_default());
        let _ = db.execute(
            "UPDATE computers SET catalog = ?1, last_seen_at = ?2 WHERE id = ?3",
            params![serde_json::to_string(&map).unwrap_or_default(), now(), computer],
        );
    };
    store_catalog(&catalog);
    if ground == "foreground" {
        let _ = server
            .db
            .lock()
            .unwrap()
            .execute("UPDATE computers SET name = ?1, os = ?2 WHERE id = ?3", params![name.trim(), os, computer]);
    }

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ServerMessage>();
    let conn = {
        let mut n = server.next_conn.lock().unwrap();
        *n += 1;
        *n
    };
    let key = (computer.clone(), ground.clone());
    if let Some((_, old)) = server.agents.lock().unwrap().insert(key.clone(), (conn, out_tx)) {
        let _ = old.send(ServerMessage::Rejected { message: "Another agent connected as this computer.".into() });
    }
    loop {
        tokio::select! {
            outgoing = out_rx.recv() => {
                let Some(message) = outgoing else { break };
                let rejected = matches!(message, ServerMessage::Rejected { .. });
                if tx.send(send(&message)).await.is_err() || rejected { break }
            }
            incoming = tokio::time::timeout(Duration::from_secs(90), rx.next()) => {
                let text = match incoming {
                    Ok(Some(Ok(Message::Text(t)))) => t.to_string(),
                    Ok(Some(Ok(Message::Ping(p)))) => { let _ = tx.send(Message::Pong(p)).await; continue }
                    Ok(Some(Ok(_))) => continue,
                    _ => break, // closed, errored, or silent for 90 s
                };
                match serde_json::from_str::<AgentMessage>(&text) {
                    Ok(AgentMessage::Ping) => {
                        let _ = server.db.lock().unwrap().execute("UPDATE computers SET last_seen_at = ?1 WHERE id = ?2", params![now(), computer]);
                        if tx.send(send(&ServerMessage::Pong)).await.is_err() { break }
                    }
                    Ok(AgentMessage::Catalog { catalog }) => store_catalog(&catalog),
                    Ok(AgentMessage::Result { run_id, status, exit_code, output, .. }) => {
                        let output: String = output.chars().rev().take(8000).collect::<Vec<_>>().into_iter().rev().collect();
                        server.record_result(&computer, &run_id, status, exit_code, &output);
                    }
                    Ok(AgentMessage::Hello { .. }) | Err(_) => {}
                }
            }
        }
    }
    {
        let mut agents = server.agents.lock().unwrap();
        if agents.get(&key).is_some_and(|(c, _)| *c == conn) {
            agents.remove(&key);
            tracing::info!("Agent disconnected: {} ({ground})", key.0);
        }
    }
    let _ = tx.close().await;
}

// ---------- MCP over HTTP ----------

async fn mcp_endpoint(State(server): State<Arc<Server>>, headers: HeaderMap, Json(message): Json<Value>) -> Response {
    let user = match server.user(&headers, &[Auth::Session, Auth::Token]) {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };
    let reply = match mcp::dispatch(&message, "cmdtrigger-server", env!("CARGO_PKG_VERSION")) {
        Ok(Some(reply)) => reply,
        Ok(None) => return StatusCode::ACCEPTED.into_response(),
        Err((id, call)) => {
            let computers = server.computers(&user).unwrap_or_default();
            let tools: Vec<(String, Value, String, CatalogEntry)> = computers
                .iter()
                .flat_map(|c| {
                    c.catalog.iter().map(move |e| {
                        let (name, tool) = mcp::tool_for(e, Some(&c.name), Some(&c.name));
                        (name, tool, c.id.clone(), e.clone())
                    })
                })
                .collect();
            match call {
                mcp::Call::ListTools => mcp::tools_list(id, tools.into_iter().map(|t| t.1).collect()),
                mcp::Call::CallTool { name, params } => match tools.into_iter().find(|t| t.0 == name) {
                    None => mcp::tool_result(id, &format!("There is no tool called {name}."), true),
                    Some((_, _, computer, entry)) => {
                        let request = RunRequest {
                            computer,
                            target: entry.id.clone(),
                            kind: Some(entry.kind),
                            params,
                            wait: Some(MAX_WAIT),
                        };
                        match server.start_run(&user, request, "mcp").await {
                            Ok(run) => {
                                let ok = run.status == RunStatus::Succeeded;
                                let mut text = format!(
                                    "\"{}\" on {}: {}.",
                                    run.target_name,
                                    run.computer_name,
                                    status_text(run.status).replace('_', " ")
                                );
                                if !run.output.trim().is_empty() {
                                    text.push_str(&format!(" Output:\n{}", run.output.trim()));
                                }
                                mcp::tool_result(id, &text, !ok)
                            }
                            Err(e) => mcp::tool_result(id, &e.1, true),
                        }
                    }
                },
            }
        }
    };
    Json(reply).into_response()
}
