//! LAN share server: a JSON API over the local `Db` under `/api/v1`, plus the
//! static web client. Started and stopped from the desktop app via
//! [`ShareHandle`]; every API request must carry the per-session token.
//!
//! Read-only by default. Write routes answer 403 until the desktop turns on
//! "allow edits" for this session ([`ShareHandle::set_allow_edits`]).

use std::convert::Infallible;
use std::future::IntoFuture;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{Response, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Router, routing::get};
use futures_util::Stream;
use polodb_core::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use crate::database::activity::{ActivityEntry, with_actor};
use crate::database::database::Db;
use crate::database::models::{Annotation, ORDER_GAP};
use crate::database::{ProjectEntry, ProjectManagement, Task, TaskManagement};
use crate::local_share::api::{
    NAME_HEADER, NewNote, NewTask, ShareInfo, TOKEN_HEADER, TaskPatch, decode_name,
};

/// The browser client: `index.html` plus the wasm-bindgen output that
/// `scripts/build-web.sh` writes next to it. Embedded in release builds; debug
/// builds read the folder from disk, so a rebuilt client shows on reload.
#[derive(rust_embed::RustEmbed)]
#[folder = "src/local_share/web/"]
struct WebAssets;

/// Preferred port; falls back to an OS-assigned one when taken.
pub const DEFAULT_PORT: u16 = 8080;
/// A running share server. Dropping it stops the server and frees the port.
pub struct ShareHandle {
    url: String,
    addr: SocketAddr,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    allow_edits: Arc<tokio::sync::watch::Sender<bool>>,
    host_name: Arc<parking_lot::RwLock<String>>,
    token: String,
}

impl ShareHandle {
    /// Full URL including the token — what the QR code encodes.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// `host:port` without the token, for display.
    pub fn display_addr(&self) -> String {
        self.url
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or_default()
            .to_string()
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The access token (part of the URL); kept across restarts with "keep link".
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Ignored when blank (browsers would show just "(host)").
    pub fn set_host_name(&self, name: &str) {
        if !name.trim().is_empty() {
            *self.host_name.write() = name.trim().to_string();
        }
    }

    /// Whether browsers may edit.
    pub fn allow_edits(&self) -> bool {
        *self.allow_edits.borrow()
    }

    /// Turn browser edits on or off. Open browsers are told right away.
    pub fn set_allow_edits(&self, on: bool) {
        self.allow_edits.send_replace(on);
    }
}

impl Drop for ShareHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Called after each successful write from a browser, so the desktop can
/// refresh. Runs on a server thread: keep it quick and non-blocking.
pub type OnWrite = Arc<dyn Fn() + Send + Sync>;

/// How to start a share. `Default` is a fresh link: new token, default port,
/// read-only.
#[derive(Default, Clone)]
pub struct ShareConfig {
    /// Reuse this token (a kept link); `None` makes a new one.
    pub token: Option<String>,
    /// Try this port first, then [`DEFAULT_PORT`], then any free port.
    pub port: Option<u16>,
    pub allow_edits: bool,
    /// How browsers see the desktop's edits in the activity log.
    pub host_name: String,
}

/// Bind the share port on every interface (so the LAN can reach it) and serve
/// `db` on a dedicated thread with its own runtime. The bind happens here, so a
/// failure is returned rather than lost in the thread.
pub fn start(db: Db, config: ShareConfig, on_write: OnWrite) -> anyhow::Result<ShareHandle> {
    let bind = |port| TcpListener::bind((Ipv4Addr::UNSPECIFIED, port));
    let listener = config
        .port
        .map_or_else(|| bind(DEFAULT_PORT), bind)
        .or_else(|_| bind(DEFAULT_PORT))
        .or_else(|_| bind(0))?;
    start_with(db, listener, config, on_write)
}

/// Like [`start`], on a listener the caller bound (tests and the demo use loopback).
pub fn start_on(db: Db, listener: TcpListener, on_write: OnWrite) -> anyhow::Result<ShareHandle> {
    start_with(db, listener, ShareConfig::default(), on_write)
}

fn start_with(
    db: Db,
    listener: TcpListener,
    config: ShareConfig,
    on_write: OnWrite,
) -> anyhow::Result<ShareHandle> {
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;

    let token = match config.token {
        Some(token) => token,
        None => new_token()?,
    };
    let host = local_ip_address::local_ip().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    let url = format!("http://{host}:{}/?t={token}", addr.port());

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let state = AppState::new(db, token.clone(), on_write);
    state.allow_edits.send_replace(config.allow_edits);
    if !config.host_name.trim().is_empty() {
        *state.host_name.write() = config.host_name;
    }
    let (allow_edits, host_name) = (state.allow_edits.clone(), state.host_name.clone());
    let app = router_with(state);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("local-share")
        .enable_all()
        .build()?;

    let thread = std::thread::Builder::new()
        .name("local-share".into())
        .spawn(move || {
            runtime.block_on(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                    return;
                };
                let serve = std::pin::pin!(axum::serve(listener, app).into_future());
                // Not `with_graceful_shutdown`: it waits for open connections, and
                // an SSE stream never closes on its own. Returning here drops the
                // runtime, which cancels every connection task.
                let _ = futures_util::future::select(serve, shutdown_rx).await;
            });
        })?;

    Ok(ShareHandle {
        url,
        addr,
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
        allow_edits,
        host_name,
        token,
    })
}

/// 128 random bits as hex.
pub fn new_token() -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("share token: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Clone)]
struct AppState {
    db: Db,
    token: Arc<str>,
    allow_edits: Arc<tokio::sync::watch::Sender<bool>>,
    host_name: Arc<parking_lot::RwLock<String>>,
    on_write: OnWrite,
}

impl AppState {
    fn new(db: Db, token: String, on_write: OnWrite) -> Self {
        Self {
            db,
            token: token.into(),
            allow_edits: Arc::new(tokio::sync::watch::Sender::new(false)),
            host_name: Arc::new(parking_lot::RwLock::new("Host".into())),
            on_write,
        }
    }
}

/// The whole share app: static client at `/`, token-guarded API under `/api/v1`.
/// Edits are off and nothing is told about writes; see [`start_on`].
pub fn router(db: Db, token: String) -> Router {
    router_with(AppState::new(db, token, Arc::new(|| {})))
}

fn router_with(state: AppState) -> Router {
    let api = Router::new()
        .route("/share", get(get_share_info))
        // Not `/activity`: ad blockers' filter lists (uBlock Origin) cancel
        // requests to paths like that, so the browser's log never loaded.
        .route("/changes", get(get_activity))
        .route("/projects", get(get_projects))
        .route("/current-project", get(get_current_project))
        .route("/tasks", get(get_tasks))
        .route("/task", axum::routing::post(create_task))
        .route(
            "/task/{task_id}",
            get(get_task).patch(update_task).delete(delete_task),
        )
        .route(
            "/task/{task_id}/annotations",
            get(get_annotations).post(add_annotation),
        )
        .route(
            "/annotation/{annotation_id}",
            axum::routing::delete(delete_annotation),
        )
        .route("/events", get(events))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_token))
        .with_state(state);

    Router::new()
        .route("/", get(index))
        .route("/app.js", get(js))
        .route("/app_bg.wasm", get(wasm))
        .nest("/api/v1", api)
}

// --- Errors ---

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

/// JSON error response: `{"error": "..."}` with the given status.
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(ErrorBody { error: self.1 })).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

// --- Auth ---

#[derive(Deserialize)]
struct TokenQuery {
    t: Option<String>,
}

async fn require_token(
    State(state): State<AppState>,
    Query(query): Query<TokenQuery>,
    req: Request,
    next: Next,
) -> Result<axum::response::Response, ApiError> {
    let header = req
        .headers()
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok());
    let given = header.or(query.t.as_deref()).unwrap_or_default();
    if constant_time_eq(given.as_bytes(), state.token.as_bytes()) {
        Ok(next.run(req).await)
    } else {
        Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "missing or invalid share token".into(),
        ))
    }
}

/// Extractor that admits a write: edits must be on for this share, and the token
/// must come in the header. (A cross-site page can't send a custom header
/// without a CORS preflight, which this server never approves.)
/// Carries the editor's name (from [`NAME_HEADER`]) for the activity log.
struct CanWrite(String);

impl axum::extract::FromRequestParts<AppState> for CanWrite {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, ApiError> {
        let header = parts
            .headers
            .get(TOKEN_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if !constant_time_eq(header.as_bytes(), state.token.as_bytes()) {
            return Err(ApiError(
                StatusCode::UNAUTHORIZED,
                format!("writes need the {TOKEN_HEADER} header"),
            ));
        }
        if !*state.allow_edits.borrow() {
            return Err(ApiError(
                StatusCode::FORBIDDEN,
                "editing is turned off for this share".into(),
            ));
        }
        let who = parts
            .headers
            .get(NAME_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(decode_name)
            .unwrap_or_else(|| "Someone".into());
        Ok(CanWrite(who))
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// --- API handlers ---

fn parse_id(raw: &str) -> Result<ObjectId, ApiError> {
    ObjectId::parse_str(raw)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, format!("invalid id: {raw}")))
}

async fn get_share_info(State(state): State<AppState>) -> Json<ShareInfo> {
    Json(ShareInfo {
        can_edit: *state.allow_edits.borrow(),
        host_name: state.host_name.read().clone(),
    })
}

/// The activity log, newest first (the last 200).
async fn get_activity(State(state): State<AppState>) -> ApiResult<Vec<ActivityEntry>> {
    Ok(Json(state.db.activity(200)?))
}

async fn get_projects(State(state): State<AppState>) -> ApiResult<Vec<ProjectEntry>> {
    Ok(Json(state.db.all_projects()?))
}

/// The project the desktop app is showing (`All` if it never picked one). The
/// web view follows it; switching projects on the desktop fires `changed`.
async fn get_current_project(State(state): State<AppState>) -> ApiResult<ProjectEntry> {
    Ok(Json(
        state.db.get_recent_project().unwrap_or(ProjectEntry::All),
    ))
}

#[derive(Deserialize)]
struct TasksQuery {
    /// A project id, or `All` / `None`. Defaults to `All`.
    project_id: Option<String>,
}

async fn get_tasks(
    State(state): State<AppState>,
    Query(query): Query<TasksQuery>,
) -> ApiResult<Vec<Task>> {
    let entry = match query.project_id.as_deref().unwrap_or("All") {
        "All" => ProjectEntry::All,
        "None" => ProjectEntry::None,
        raw => state
            .db
            .one_project(parse_id(raw)?)?
            .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "project not found".into()))?,
    };
    Ok(Json(state.db.get_tasks(entry)?))
}

async fn get_task(State(state): State<AppState>, Path(task_id): Path<String>) -> ApiResult<Task> {
    state
        .db
        .one_task(parse_id(&task_id)?)?
        .map(Json)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "task not found".into()))
}

async fn get_annotations(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> ApiResult<Vec<Annotation>> {
    Ok(Json(state.db.get_annotations(parse_id(&task_id)?)?))
}

/// Server-Sent Events: one `changed` event per logical DB write, and when edits
/// are turned on or off. Clients refetch what they show; there's no payload.
async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let changes = (state.db.subscribe_changes(), state.allow_edits.subscribe());
    // Something to send right away: browsers only report the stream open once
    // data arrives, so without it they sat at "connecting" until the first
    // change. It also tells them to retry after 2 s if the stream drops.
    let hello = futures_util::stream::once(async {
        Ok::<_, Infallible>(Event::default().retry(std::time::Duration::from_secs(2)))
    });
    let changed = futures_util::stream::unfold(changes, |(mut db, mut edits)| async move {
        tokio::select! {
            r = db.changed() => r.ok()?,
            r = edits.changed() => r.ok()?,
        }
        db.mark_unchanged();
        edits.mark_unchanged();
        Some((Ok(Event::default().event("changed").data("")), (db, edits)))
    });
    Sse::new(futures_util::StreamExt::chain(hello, changed)).keep_alive(KeepAlive::default())
}

// --- Write handlers (behind `CanWrite`) ---

fn not_found(what: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("{what} not found"))
}

fn bad_request(msg: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

/// Adds the task at the end of the desktop's current project.
async fn create_task(
    CanWrite(who): CanWrite,
    State(state): State<AppState>,
    Json(body): Json<NewTask>,
) -> ApiResult<ObjectId> {
    // Attributes every write below to `who` in the activity log.
    with_actor(&who, || {
        let title = body.title.trim();
        if title.is_empty() {
            return Err(bad_request("title is empty"));
        }
        let project = state.db.get_recent_project().unwrap_or(ProjectEntry::All);
        let last = state
            .db
            .get_tasks(project.clone())?
            .iter()
            .map(|t| t.order)
            .max()
            .unwrap_or(0);
        let id = state.db.create_task(Task {
            title: title.to_string(),
            project_id: project.get_id(),
            order: last + ORDER_GAP,
            ..Default::default()
        })?;
        (state.on_write)();
        Ok(Json(id))
    })
}

async fn update_task(
    CanWrite(who): CanWrite,
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(patch): Json<TaskPatch>,
) -> ApiResult<ObjectId> {
    // Attributes every write below to `who` in the activity log.
    with_actor(&who, || {
        let id = parse_id(&task_id)?;
        if patch.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            return Err(bad_request("title is empty"));
        }
        let TaskPatch {
            title,
            details,
            priority,
            status,
        } = patch;
        if title.is_some() || details.is_some() || priority.is_some() {
            state
                .db
                .modify_task(id, &mut |task| {
                    if let Some(title) = &title {
                        task.title = title.trim().to_string();
                    }
                    if let Some(details) = &details {
                        task.details = details.clone();
                    }
                    if let Some(priority) = &priority {
                        task.priority = priority.clone();
                    }
                })?
                .ok_or_else(|| not_found("task"))?;
        }
        // Through `set_status`, so completing a recurring task schedules the next one.
        if let Some(status) = &status {
            state
                .db
                .set_status(id, status)?
                .ok_or_else(|| not_found("task"))?;
        }
        (state.on_write)();
        Ok(Json(id))
    })
}

async fn delete_task(
    CanWrite(who): CanWrite,
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> ApiResult<ObjectId> {
    // Attributes every write below to `who` in the activity log.
    with_actor(&who, || {
        let id = parse_id(&task_id)?;
        if state.db.one_task(id)?.is_none() {
            return Err(not_found("task"));
        }
        state.db.delete_task(id)?;
        (state.on_write)();
        Ok(Json(id))
    })
}

async fn add_annotation(
    CanWrite(who): CanWrite,
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(body): Json<NewNote>,
) -> ApiResult<ObjectId> {
    // Attributes every write below to `who` in the activity log.
    with_actor(&who, || {
        let task_id = parse_id(&task_id)?;
        let content = body.content.trim();
        if content.is_empty() {
            return Err(bad_request("note is empty"));
        }
        if state.db.one_task(task_id)?.is_none() {
            return Err(not_found("task"));
        }
        let id = state.db.add_annotation(Annotation {
            id: ObjectId::new(),
            task_id,
            content: content.to_string(),
            created_at: bson::DateTime::now(),
            author: Some(who.clone()),
        })?;
        (state.on_write)();
        Ok(Json(id))
    })
}

async fn delete_annotation(
    CanWrite(who): CanWrite,
    State(state): State<AppState>,
    Path(annotation_id): Path<String>,
) -> ApiResult<ObjectId> {
    // Attributes every write below to `who` in the activity log.
    with_actor(&who, || {
        let id = parse_id(&annotation_id)?;
        state.db.delete_annotation(id)?;
        (state.on_write)();
        Ok(Json(id))
    })
}

// --- Static client ---

async fn index() -> Response<Body> {
    if WebAssets::get("app_bg.wasm").is_none() {
        // A build without the client would otherwise load a blank canvas.
        return Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(
                "<h1>FastTask web client not built</h1>\
                 <p>Run <code>scripts/build-web.sh</code>, then rebuild the app.</p>",
            ))
            .unwrap();
    }
    serve_asset("index.html")
}

async fn js() -> Response<Body> {
    serve_asset("app.js")
}

async fn wasm() -> Response<Body> {
    serve_asset("app_bg.wasm")
}

fn serve_asset(name: &str) -> Response<Body> {
    let Some(file) = WebAssets::get(name) else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404"))
            .unwrap();
    };
    let mime = match name.rsplit('.').next() {
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        _ => "text/html; charset=utf-8",
    };
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        // Revalidate every load, so a rebuilt client is never masked by the cache.
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(file.data.into_owned()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::models::{Priority, Project, TaskStatus};
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    const TOKEN: &str = "secret";

    // TempDir first so it outlives Db.
    fn setup() -> (TempDir, Db, Router) {
        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        let app = router(db.clone(), TOKEN.into());
        (dir, db, app)
    }

    async fn get(app: &Router, uri: &str, token: Option<&str>) -> (StatusCode, Vec<u8>) {
        let mut req = Request::get(uri);
        if let Some(t) = token {
            req = req.header(TOKEN_HEADER, t);
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, body.to_vec())
    }

    fn task(title: &str, project_id: Option<ObjectId>) -> Task {
        Task {
            title: title.into(),
            project_id,
            order: 1000,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn api_requires_token() {
        let (_dir, _db, app) = setup();
        assert_eq!(
            get(&app, "/api/v1/tasks", None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get(&app, "/api/v1/tasks", Some("wrong")).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get(&app, "/api/v1/tasks", Some(TOKEN)).await.0,
            StatusCode::OK
        );
        // Query param works too (EventSource can't send headers).
        assert_eq!(
            get(&app, &format!("/api/v1/tasks?t={TOKEN}"), None).await.0,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn tasks_filter_by_project() {
        let (_dir, db, app) = setup();
        let project = Project::new("p", None);
        let pid = db.create_project(project).unwrap();
        db.create_task(task("in", Some(pid))).unwrap();
        db.create_task(task("out", None)).unwrap();

        let (status, body) = get(
            &app,
            &format!("/api/v1/tasks?project_id={pid}"),
            Some(TOKEN),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let tasks: Vec<Task> = serde_json_from(&body);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "in");

        let (_, body) = get(&app, "/api/v1/tasks", Some(TOKEN)).await;
        assert_eq!(serde_json_from::<Vec<Task>>(&body).len(), 2);

        let (_, body) = get(&app, "/api/v1/projects", Some(TOKEN)).await;
        assert_eq!(serde_json_from::<Vec<ProjectEntry>>(&body).len(), 1);
    }

    #[tokio::test]
    async fn task_and_annotations_by_id() {
        let (_dir, db, app) = setup();
        let id = db.create_task(task("one", None)).unwrap();
        db.add_annotation(Annotation {
            id: ObjectId::new(),
            task_id: id,
            content: "note".into(),
            created_at: polodb_core::bson::DateTime::now(),
            author: None,
        })
        .unwrap();

        let (status, body) = get(&app, &format!("/api/v1/task/{id}"), Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json_from::<Task>(&body).title, "one");

        let (_, body) = get(&app, &format!("/api/v1/task/{id}/annotations"), Some(TOKEN)).await;
        assert_eq!(serde_json_from::<Vec<Annotation>>(&body)[0].content, "note");
    }

    #[tokio::test]
    async fn bad_ids_and_missing_tasks_are_errors_not_panics() {
        let (_dir, _db, app) = setup();
        let (status, body) = get(&app, "/api/v1/task/nope", Some(TOKEN)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(String::from_utf8(body).unwrap().contains("\"error\""));
        let missing = ObjectId::new();
        assert_eq!(
            get(&app, &format!("/api/v1/task/{missing}"), Some(TOKEN))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(
                &app,
                &format!("/api/v1/tasks?project_id={missing}"),
                Some(TOKEN)
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn current_project_follows_the_desktop() {
        let (_dir, db, app) = setup();
        let (status, body) = get(&app, "/api/v1/current-project", Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json_from::<ProjectEntry>(&body), ProjectEntry::All);

        let project = Project::new("p", None);
        db.create_project(project.clone()).unwrap();
        let rx = db.subscribe_changes();
        let before = *rx.borrow();
        db.save_current_project(ProjectEntry::Project(project.clone()))
            .unwrap();
        assert_eq!(*rx.borrow(), before + 1, "switching project notifies");
        let (_, body) = get(&app, "/api/v1/current-project", Some(TOKEN)).await;
        assert_eq!(
            serde_json_from::<ProjectEntry>(&body),
            ProjectEntry::Project(project)
        );
    }

    #[tokio::test]
    async fn index_is_public_and_not_cached() {
        let (_dir, _db, app) = setup();
        let res = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        // 200 with the client built, 503 with the "not built" page — never 401.
        assert!(
            res.status() == StatusCode::OK || res.status() == StatusCode::SERVICE_UNAVAILABLE,
            "{}",
            res.status()
        );
        if res.status() == StatusCode::OK {
            assert_eq!(res.headers()[header::CACHE_CONTROL], "no-cache");
        }
    }

    /// Share with edits on; the counter counts `on_write` calls.
    fn setup_editable() -> (TempDir, Db, Router, Arc<std::sync::atomic::AtomicUsize>) {
        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = writes.clone();
        let state = AppState::new(
            db.clone(),
            TOKEN.into(),
            Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        );
        state.allow_edits.send_replace(true);
        (dir, db, router_with(state), writes)
    }

    /// Send `method uri` with a JSON body and the token header.
    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, Vec<u8>) {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(TOKEN_HEADER, TOKEN)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, body.to_vec())
    }

    #[tokio::test]
    async fn browser_edits_are_logged_under_their_name() {
        let (_dir, db, app, _writes) = setup_editable();
        db.set_activity_logging(true);
        let res = app
            .clone()
            .oneshot(
                Request::post("/api/v1/task")
                    .header(TOKEN_HEADER, TOKEN)
                    .header(NAME_HEADER, crate::local_share::api::encode_name("José"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"title":"Buy milk"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // No name header: still allowed, logged as "Someone".
        send(
            &app,
            "POST",
            "/api/v1/task",
            serde_json::json!({"title": "Eggs"}),
        )
        .await;
        // The desktop's own write (no actor) has no name.
        db.create_task(task("Bread", None)).unwrap();

        let (status, body) = get(&app, "/api/v1/changes", Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        let log: Vec<(Option<String>, String)> = serde_json_from::<Vec<ActivityEntry>>(&body)
            .into_iter()
            .map(|e| (e.who, e.what))
            .collect();
        assert_eq!(
            log,
            vec![
                (None, "added “Bread”".into()),
                (Some("Someone".into()), "added “Eggs”".into()),
                (Some("José".into()), "added “Buy milk”".into()),
            ]
        );
    }

    #[test]
    fn a_kept_link_reuses_its_token_and_port() {
        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        let first = start_with(
            db.clone(),
            TcpListener::bind("127.0.0.1:0").unwrap(),
            ShareConfig::default(),
            Arc::new(|| {}),
        )
        .unwrap();
        let (token, port) = (first.token().to_string(), first.port());
        first.set_host_name("  ");
        drop(first);

        let again = start_with(
            db,
            TcpListener::bind(("127.0.0.1", port)).unwrap(),
            ShareConfig {
                token: Some(token.clone()),
                allow_edits: true,
                host_name: "Casey".into(),
                ..Default::default()
            },
            Arc::new(|| {}),
        )
        .unwrap();
        assert_eq!(again.token(), token);
        assert!(again.url().ends_with(&format!(":{port}/?t={token}")));
        assert!(again.allow_edits());
        let info: ShareInfo = reqwest::blocking::Client::new()
            .get(format!("http://127.0.0.1:{port}/api/v1/share"))
            .header(TOKEN_HEADER, &token)
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(
            info,
            ShareInfo {
                can_edit: true,
                host_name: "Casey".into()
            }
        );
    }

    #[tokio::test]
    async fn writes_are_forbidden_until_edits_are_on() {
        let (_dir, db, app) = setup();
        let id = db.create_task(task("t", None)).unwrap();
        let uri = format!("/api/v1/task/{id}");
        let (status, _) = send(&app, "DELETE", &uri, serde_json::json!(null)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = send(
            &app,
            "POST",
            "/api/v1/task",
            serde_json::json!({"title": "x"}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(db.one_task(id).unwrap().is_some());

        let (_, body) = get(&app, "/api/v1/share", Some(TOKEN)).await;
        assert_eq!(
            serde_json_from::<ShareInfo>(&body),
            ShareInfo {
                can_edit: false,
                host_name: "Host".into()
            }
        );
    }

    #[tokio::test]
    async fn writes_need_the_header_token_not_the_query() {
        let (_dir, db, app, writes) = setup_editable();
        let id = db.create_task(task("t", None)).unwrap();
        let res = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/v1/task/{id}?t={TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert!(db.one_task(id).unwrap().is_some());
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn create_lands_at_the_end_of_the_current_project() {
        let (_dir, db, app, writes) = setup_editable();
        let project = Project::new("p", None);
        let pid = db.create_project(project.clone()).unwrap();
        db.save_current_project(ProjectEntry::Project(project))
            .unwrap();
        db.create_task(Task {
            order: 5000,
            ..task("existing", Some(pid))
        })
        .unwrap();

        let (status, body) = send(
            &app,
            "POST",
            "/api/v1/task",
            serde_json::json!({"title": "  new  "}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let created = db
            .one_task(serde_json_from::<ObjectId>(&body))
            .unwrap()
            .unwrap();
        assert_eq!(created.title, "new");
        assert_eq!(created.project_id, Some(pid));
        assert_eq!(created.order, 5000 + ORDER_GAP);
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 1);

        let (status, _) = send(
            &app,
            "POST",
            "/api/v1/task",
            serde_json::json!({"title": " "}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_changes_only_the_given_fields() {
        let (_dir, db, app, _writes) = setup_editable();
        let id = db
            .create_task(Task {
                details: "keep".into(),
                ..task("old", None)
            })
            .unwrap();
        let (status, _) = send(
            &app,
            "PATCH",
            &format!("/api/v1/task/{id}"),
            serde_json::json!({"title": "new", "priority": "Urgent"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let t = db.one_task(id).unwrap().unwrap();
        assert_eq!(
            (t.title.as_str(), t.details.as_str(), t.priority),
            ("new", "keep", Priority::Urgent)
        );

        let (status, _) = send(
            &app,
            "PATCH",
            &format!("/api/v1/task/{}", ObjectId::new()),
            serde_json::json!({"title": "x"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn completing_a_recurring_task_schedules_one_next_occurrence() {
        let (_dir, db, app, _writes) = setup_editable();
        let id = db
            .create_task(Task {
                recurrence: Some(crate::database::models::Recurrence::Weekly),
                ..task("water plants", None)
            })
            .unwrap();
        for _ in 0..2 {
            let (status, _) = send(
                &app,
                "PATCH",
                &format!("/api/v1/task/{id}"),
                serde_json::json!({"status": "Completed"}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        let tasks = db.get_tasks(ProjectEntry::All).unwrap();
        assert_eq!(tasks.len(), 2, "one next occurrence, not one per request");
        assert!(
            tasks
                .iter()
                .any(|t| t.id != id && t.status == TaskStatus::NotStarted)
        );
    }

    #[tokio::test]
    async fn delete_task_and_notes() {
        let (_dir, db, app, writes) = setup_editable();
        let id = db.create_task(task("t", None)).unwrap();

        let (status, body) = send(
            &app,
            "POST",
            &format!("/api/v1/task/{id}/annotations"),
            serde_json::json!({"content": "hello"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let note_id = serde_json_from::<ObjectId>(&body);
        let notes = db.get_annotations(id).unwrap();
        assert_eq!(notes[0].content, "hello");
        assert_eq!(
            notes[0].author.as_deref(),
            Some("Someone"),
            "no name header"
        );

        let (status, _) = send(
            &app,
            "DELETE",
            &format!("/api/v1/annotation/{note_id}"),
            serde_json::json!(null),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(db.get_annotations(id).unwrap().is_empty());

        let (status, _) = send(
            &app,
            "DELETE",
            &format!("/api/v1/task/{id}"),
            serde_json::json!(null),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(db.one_task(id).unwrap().is_none());
        let (status, _) = send(
            &app,
            "DELETE",
            &format!("/api/v1/task/{id}"),
            serde_json::json!(null),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn start_serves_and_stop_frees_the_port() {
        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        // Loopback only: binding 0.0.0.0 from a test can trigger the macOS firewall prompt.
        let handle = start_on(
            db,
            TcpListener::bind("127.0.0.1:0").unwrap(),
            Arc::new(|| {}),
        )
        .unwrap();
        let port = handle.port();
        let token = handle.url().rsplit("t=").next().unwrap().to_string();
        assert_eq!(token.len(), 32);

        let res = reqwest::blocking::Client::new()
            .get(format!("http://127.0.0.1:{port}/api/v1/tasks"))
            .header(TOKEN_HEADER, &token)
            .send()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);

        drop(handle);
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "server still accepting after stop"
        );
    }

    /// The stream sends data at once, before any write: browsers only fire
    /// `open` once data arrives, and showed "connecting" until the first change.
    #[test]
    fn events_stream_opens_immediately() {
        use std::io::Read;
        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        let handle = start_on(
            db,
            TcpListener::bind("127.0.0.1:0").unwrap(),
            Arc::new(|| {}),
        )
        .unwrap();
        let token = handle.token().to_string();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", handle.port())).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        use std::io::Write;
        write!(
            stream,
            "GET /api/v1/events?t={token} HTTP/1.1\r\nHost: x\r\n\r\n"
        )
        .unwrap();
        // Read until the first SSE field shows up (head and body may arrive
        // in separate reads).
        let mut got = String::new();
        let mut buf = [0u8; 512];
        while !got.contains("retry: 2000") {
            let n = stream
                .read(&mut buf)
                .expect("no event data within 3 s of connecting");
            assert!(n > 0, "stream closed: {got}");
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(got.starts_with("HTTP/1.1 200"), "{got}");
    }

    /// A write pushes a `changed` event, and stopping doesn't hang on the
    /// still-open SSE stream (the reason `start_on` avoids graceful shutdown).
    #[test]
    fn events_push_changes_and_stop_with_open_stream() {
        use std::io::{BufRead, BufReader};

        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        let handle = start_on(
            db.clone(),
            TcpListener::bind("127.0.0.1:0").unwrap(),
            Arc::new(|| {}),
        )
        .unwrap();
        let token = handle.url().rsplit("t=").next().unwrap().to_string();

        let res = reqwest::blocking::Client::new()
            .get(format!(
                "http://127.0.0.1:{}/api/v1/events?t={token}",
                handle.port()
            ))
            .send()
            .unwrap();
        assert_eq!(res.status().as_u16(), 200);

        db.create_task(task("new", None)).unwrap();
        let mut lines = BufReader::new(res).lines();
        let first = lines
            .by_ref()
            .map(|l| l.unwrap())
            .find(|l| l.starts_with("event:"))
            .unwrap();
        assert_eq!(first, "event: changed");

        // Turning edits on tells open browsers too.
        handle.set_allow_edits(true);
        let next_event = lines
            .by_ref()
            .map(|l| l.unwrap())
            .find(|l| l.starts_with("event:"))
            .unwrap();
        assert_eq!(next_event, "event: changed");

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(handle);
            let _ = done_tx.send(());
        });
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("stopping the share hung on an open event stream");
        drop(lines);
    }

    fn serde_json_from<T: serde::de::DeserializeOwned>(body: &[u8]) -> T {
        // axum's Json extractor uses serde_json; reuse it via a tiny round-trip.
        axum::Json::<T>::from_bytes(body).unwrap().0
    }
}
