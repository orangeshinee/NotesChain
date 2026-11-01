//! HTTP 接口层：token 鉴权、文件读写、待办汇总与回写。

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::AppState;
use crate::todo_write::{self, LocateError};

/// 每日文件骨架的权威副本（PRD 3.1.4），编译期内嵌，不读运行时文件。
const DAILY_TEMPLATE: &str = include_str!("../docs/templates/daily-note.md");
const INDEX_HTML: &str = include_str!("../web/index.html");

pub fn router(state: Arc<AppState>) -> Router {
    let api = Router::new()
        .route("/api/files", get(list_files))
        .route("/api/files/*path", get(read_file).put(save_file))
        .route("/api/todos", get(list_todos).patch(patch_todo))
        .route("/api/daily", post(ensure_daily))
        .route("/api/index/rebuild", post(rebuild_index))
        .route("/api/git/status", get(git_status))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth));

    Router::new()
        .route("/", get(root))
        .merge(api)
        .with_state(state)
}

// ---------------------------------------------------------------- 错误与鉴权

pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }

    fn bad_path(err: impl std::fmt::Display) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_path", err.to_string())
    }

    fn internal(err: impl std::fmt::Display) -> Self {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": { "code": self.code, "message": self.message } })),
        )
            .into_response()
    }
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|part| part.trim().strip_prefix("nc_token="))
        .next()
        .map(|v| v.to_string())
}

async fn auth(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    match cookie_token(req.headers()) {
        Some(token) if token == st.token => next.run(req).await,
        _ => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "token 缺失或已失效，请重新打开启动地址",
        )
        .into_response(),
    }
}

#[derive(Deserialize)]
struct RootQuery {
    token: Option<String>,
}

async fn root(State(st): State<Arc<AppState>>, Query(q): Query<RootQuery>, headers: HeaderMap) -> Response {
    if let Some(token) = q.token {
        if token == st.token {
            let mut resp = Redirect::temporary("/").into_response();
            let value = format!("nc_token={}; Path=/; HttpOnly; SameSite=Strict", st.token);
            if let Ok(value) = HeaderValue::from_str(&value) {
                resp.headers_mut().insert(header::SET_COOKIE, value);
            }
            return resp;
        }
        return unauthorized_page();
    }
    if cookie_token(&headers).as_deref() == Some(st.token.as_str()) {
        return Html(INDEX_HTML).into_response();
    }
    unauthorized_page()
}

fn unauthorized_page() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Html("<h1>401</h1><p>服务已重启或 token 无效，请重新打开启动时打印的地址。</p>"),
    )
        .into_response()
}

// ---------------------------------------------------------------- 文件

#[derive(Serialize)]
struct FileItem {
    path: String,
    mtime: u64,
    size: u64,
    title: String,
}

async fn list_files(State(st): State<Arc<AppState>>) -> Json<Value> {
    let index = st.index.read().await;

    // 日记按 年 → 月 分组；其余归入 note 组
    let mut years: BTreeMap<String, BTreeMap<String, Vec<FileItem>>> = BTreeMap::new();
    let mut notes: Vec<FileItem> = Vec::new();

    for file in &index.files {
        let item = FileItem {
            path: file.path.clone(),
            mtime: file.mtime_ms,
            size: file.size,
            title: file.path.clone(),
        };
        match &file.diary_date {
            Some(date) => {
                let year = date[..4].to_string();
                let month = date[5..7].to_string();
                years.entry(year).or_default().entry(month).or_default().push(item);
            }
            None => notes.push(item),
        }
    }

    let mut diary_groups = Vec::new();
    for (year, months) in years {
        let mut month_entries = Vec::new();
        for (month, items) in months {
            month_entries.push(json!({ "month": month, "entries": items }));
        }
        diary_groups.push(json!({ "year": year, "months": month_entries }));
    }

    Json(json!({
        "groups": [
            { "kind": "diary", "years": diary_groups },
            { "kind": "note", "entries": notes },
        ],
        "warnings": index.warnings,
    }))
}

async fn read_file(State(st): State<Arc<AppState>>, AxumPath(path): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    let abs = st.resolve(&path).map_err(ApiError::bad_path)?;
    if !abs.is_file() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "文件不存在"));
    }
    let content = std::fs::read_to_string(&abs).map_err(ApiError::internal)?;
    let mtime = mtime_ms(&abs);
    Ok(Json(json!({
        "path": st.relative(&abs),
        "content": content,
        "version": crate::index::version_of(mtime, &content),
    })))
}

#[derive(Deserialize)]
struct SaveRequest {
    content: String,
    version: String,
}

async fn save_file(
    State(st): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
    Json(body): Json<SaveRequest>,
) -> Result<Json<Value>, ApiError> {
    let abs = st.resolve(&path).map_err(ApiError::bad_path)?;
    let rel = st.relative(&abs);
    let lock = st.lock_for(&rel).await;
    let _guard = lock.lock().await;

    let current = std::fs::read_to_string(&abs).unwrap_or_default();
    let current_version = crate::index::version_of(mtime_ms(&abs), &current);
    if current_version != body.version {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "文件已被外部修改，请重新加载或强制覆盖",
        ));
    }

    todo_write::atomic_write(&abs, &body.content).map_err(ApiError::internal)?;
    st.index.write().await.upsert(&st.root, &rel).map_err(ApiError::internal)?;

    let mtime = mtime_ms(&abs);
    Ok(Json(json!({
        "path": rel,
        "version": crate::index::version_of(mtime, &body.content),
    })))
}

// ---------------------------------------------------------------- 待办

async fn list_todos(State(st): State<Arc<AppState>>) -> Json<Value> {
    let index = st.index.read().await;
    let rows = index.open_todos();
    Json(json!({ "total": rows.len(), "items": rows }))
}

#[derive(Deserialize)]
struct PatchTodoRequest {
    path: String,
    line: usize,
    raw_line: String,
    /// 版本号仅作参考：不一致时仍走三级解析重新定位（PRD 3.2.3）
    #[serde(rename = "version", default)]
    _version: Option<String>,
    done: bool,
}

async fn patch_todo(
    State(st): State<Arc<AppState>>,
    Json(body): Json<PatchTodoRequest>,
) -> Result<Json<Value>, ApiError> {
    let abs = st.resolve(&body.path).map_err(ApiError::bad_path)?;
    let rel = st.relative(&abs);
    let lock = st.lock_for(&rel).await;
    let _guard = lock.lock().await;

    let content = std::fs::read_to_string(&abs).map_err(|e| {
        ApiError::new(StatusCode::NOT_FOUND, "not_found", format!("读取文件失败: {e}"))
    })?;

    // 版本号不一致不直接拒绝，交给三级解析重新定位（PRD 3.2.3 并发守卫）
    let located = todo_write::locate(&content, body.line, &body.raw_line)
        .map_err(|err: LocateError| {
            ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, err.code(), err.message())
        })?;

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let original = lines.get(located).cloned().unwrap_or_default();
    lines[located] = todo_write::toggle_line(&original, body.done, &now);

    let mut updated = lines.join("\n");
    if content.ends_with('\n') {
        updated.push('\n');
    }

    todo_write::atomic_write(&abs, &updated).map_err(ApiError::internal)?;
    st.index.write().await.upsert(&st.root, &rel).map_err(ApiError::internal)?;

    let mtime = mtime_ms(&abs);
    Ok(Json(json!({
        "path": rel,
        "line": located + 1,
        "version": crate::index::version_of(mtime, &updated),
    })))
}

// ---------------------------------------------------------------- 每日文件 / 索引 / git

async fn ensure_daily(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let rel = format!("{}/{}/{}.md", &today[..4], &today[5..7], today);
    let abs = st.resolve(&rel).map_err(ApiError::bad_path)?;

    let lock = st.lock_for(&rel).await;
    let _guard = lock.lock().await;

    if abs.is_file() {
        let content = std::fs::read_to_string(&abs).map_err(ApiError::internal)?;
        let mtime = mtime_ms(&abs);
        return Ok(Json(json!({
            "path": rel,
            "created": false,
            "content": content,
            "version": crate::index::version_of(mtime, &content),
        })));
    }

    let content = DAILY_TEMPLATE.replace("YYYY-MM-DD", &today);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
    }
    todo_write::atomic_write(&abs, &content).map_err(ApiError::internal)?;
    st.index.write().await.upsert(&st.root, &rel).map_err(ApiError::internal)?;

    Ok(Json(json!({
        "path": rel,
        "created": true,
        "content": content,
        "version": crate::index::version_of(mtime_ms(&abs), &content),
    })))
}

async fn rebuild_index(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let warnings = st.rebuild_index().await.map_err(ApiError::internal)?;
    let files = st.index.read().await.files.len();
    Ok(Json(json!({ "rebuilt": true, "files": files, "warnings": warnings })))
}

async fn git_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    let root = st.root.clone();
    let result = tokio::task::spawn_blocking(move || probe_git(&root)).await;
    Json(match result {
        Ok(value) => value,
        Err(err) => json!({ "available": false, "reason": format!("检测失败: {err}") }),
    })
}

fn probe_git(root: &std::path::Path) -> Value {
    let version = std::process::Command::new("git").arg("--version").output();
    let Ok(version) = version else {
        return json!({ "available": false, "reason": "未检测到 git，版本历史未启用" });
    };
    if !version.status.success() {
        return json!({ "available": false, "reason": "未检测到 git，版本历史未启用" });
    }

    let inside = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .output();
    let is_repo = inside.map(|o| o.status.success()).unwrap_or(false);

    let last = std::process::Command::new("git")
        .args(["log", "-1", "--format=%cI"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let pending = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0);

    json!({
        "available": true,
        "is_repo": is_repo,
        "last_commit": last,
        "pending_files": pending,
        "note": "仅本地版本历史，无远程备份",
    })
}

fn mtime_ms(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
