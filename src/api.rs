//! HTTP 接口层：token 鉴权、文件读写、待办汇总与回写。

use std::collections::BTreeMap;
use std::path::PathBuf;
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
use crate::write;

/// 每日文件骨架的权威副本（PRD 3.1.4），编译期内嵌，不读运行时文件。
const DAILY_TEMPLATE: &str = include_str!("../docs/templates/daily-note.md");
const NOTE_TEMPLATE: &str = include_str!("../docs/templates/note.md");
const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
const EDITOR_JS: &str = include_str!("../web/dist/editor.js");

/// 渲染模板占位符：{{TITLE}} / {{DATE}} / {{DATETIME}}
fn render_template(tpl: &str, title: &str, date: &str, datetime: &str) -> String {
    tpl.replace("{{TITLE}}", title)
        .replace("{{DATE}}", date)
        .replace("{{DATETIME}}", datetime)
}

fn render_template_full(tpl: &str, title: &str, date: &str, datetime: &str) -> String {
    render_template(tpl, title, date, datetime).replace("{{TAGS}}", "[]")
}

/// 路径形如 `YYYY/MM/YYYY-MM-DD.md` 视为日记
fn diary_date_from_path(rel: &str) -> Option<String> {
    let parts: Vec<&str> = rel.trim_end_matches(".md").split('/').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 {
        return None;
    }
    if !parts[0].chars().all(|c| c.is_ascii_digit()) || !parts[1].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let date = parts[2];
    if date.len() == 10 && date.starts_with(&format!("{}-{}-", parts[0], parts[1])) {
        Some(date.to_string())
    } else {
        None
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/api/files", get(list_files))
        .route("/api/files/*path", get(read_file).put(save_file).delete(delete_file))
        .route("/api/todos", get(list_todos).patch(patch_todo))
        .route("/api/todos/append", post(append_todo))
        .route("/api/notes/append", post(append_note))
        .route("/api/notes/new", post(create_note))
        .route("/api/templates", get(list_templates))
        .route("/api/schedule/append", post(append_schedule))
        .route("/api/tags", get(list_tags))
        .route("/api/search", get(search))
        .route("/api/schedule", get(schedule_view))
        .route("/api/revision", get(revision))
        .route("/api/config", get(config))
        .route("/api/asset", get(asset))
        .route("/api/daily", post(ensure_daily))
        .route("/api/index/rebuild", post(rebuild_index))
        .route("/api/git/status", get(git_status))
        .route("/api/git/log", get(git_log))
        .route("/api/git/show", get(git_show))
        .route("/api/git/restore", post(git_restore))
        .route("/api/backlinks", get(backlinks))
        .route("/api/links", get(outgoing_links))
        .route("/api/migration/todo", get(migration_todos))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/editor.js", get(editor_js))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth));

    Router::new()
        .route("/", get(root))
        .merge(protected)
        .with_state(state)
}

// 静态资源一律 no-cache：升级后不会拿旧的 app.js / editor.js 混搭新接口
async fn app_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        APP_JS,
    )
}

async fn style_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        STYLE_CSS,
    )
}

async fn editor_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        EDITOR_JS,
    )
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

/// CLI 鉴权：`Authorization: Bearer <token>`（设计文档 §5，token 与 Cookie 是同一启动 token）
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 恒定时间比较 token：避免 `==` 按字符提前返回泄露匹配前缀的时序。
/// 本地回环 + 64 位随机 token 下风险极低，但成本也极低，统一走这里。
fn token_eq(provided: Option<&str>, expected: &str) -> bool {
    let Some(p) = provided else { return false };
    let (a, b) = (p.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn auth(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    // --no-token：进程启动时 token 为空串，直接放行（仅建议个人独占机器）
    if st.token.is_empty() {
        return next.run(req).await;
    }
    let cookie_ok = token_eq(cookie_token(req.headers()).as_deref(), &st.token);
    let bearer_ok = token_eq(bearer_token(req.headers()).as_deref(), &st.token);
    // 用 `|` 而非 `||`：两边都求值，避免短路泄漏「哪种凭证命中」
    if cookie_ok | bearer_ok {
        return next.run(req).await;
    }
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "token 缺失或已失效，请重新打开启动地址",
    )
    .into_response()
}

#[derive(Deserialize)]
struct RootQuery {
    token: Option<String>,
}

async fn root(State(st): State<Arc<AppState>>, Query(q): Query<RootQuery>, headers: HeaderMap) -> Response {
    // --no-token：不鉴权，直接返回页面
    if st.token.is_empty() {
        return Html(INDEX_HTML).into_response();
    }
    let has_valid_cookie = token_eq(cookie_token(&headers).as_deref(), &st.token);
    if let Some(token) = q.token {
        // 已带有效 Cookie 时，再来 ?token= 一律拒绝重复引导（PRD 9.1），直接回干净地址
        if has_valid_cookie {
            return Redirect::temporary("/").into_response();
        }
        if token_eq(Some(token.as_str()), &st.token) {
            let mut resp = Redirect::temporary("/").into_response();
            let value = format!("nc_token={}; Path=/; HttpOnly; SameSite=Strict", st.token);
            if let Ok(value) = HeaderValue::from_str(&value) {
                resp.headers_mut().insert(header::SET_COOKIE, value);
            }
            return resp;
        }
        return unauthorized_page();
    }
    if token_eq(cookie_token(&headers).as_deref(), &st.token) {
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
    /// 未完成待办数（文件树里显示）
    todos: usize,
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
            todos: file.todos.iter().filter(|t| !t.done).count(),
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
    let content = content.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(content);
    let mtime = mtime_ms(&abs);
    let version = crate::index::version_of_text(mtime, &content);
    let uses_crlf = content.contains("\r\n");
    let normalized = if uses_crlf {
        content.replace("\r\n", "\n")
    } else {
        content
    };
    Ok(Json(json!({
        "path": st.relative(&abs),
        "content": normalized,
        "eol": if uses_crlf { "crlf" } else { "lf" },
        "version": version,
    })))
}

#[derive(Deserialize)]
struct SaveRequest {
    content: String,
    version: String,
    /// 原文件行尾：lf / crlf（编辑器统一按 LF 编辑，保存时还原）
    #[serde(default)]
    eol: Option<String>,
}

#[derive(Deserialize)]
struct ForceQuery {
    #[serde(default)]
    force: Option<bool>,
}

async fn save_file(
    State(st): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
    Query(q): Query<ForceQuery>,
    Json(body): Json<SaveRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_markdown(&path, "保存")?;
    let abs = st.resolve(&path).map_err(ApiError::bad_path)?;
    let rel = st.relative(&abs);
    let lock = st.lock_for(&rel).await;
    let _guard = lock.lock().await;

    let current = std::fs::read_to_string(&abs).unwrap_or_default();
    let current_version = crate::index::version_of_text(mtime_ms(&abs), &current);
    if current_version != body.version && !q.force.unwrap_or(false) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "文件已被外部修改，请重新加载或强制覆盖",
        ));
    }

    if q.force.unwrap_or(false) {
        if let Some(parent) = abs.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
            }
        }
    }
    let had_bom = current.starts_with('\u{feff}');
    let mut outgoing = if body.eol.as_deref() == Some("crlf") {
        body.content.replace('\n', "\r\n")
    } else {
        body.content.clone()
    };
    if had_bom && !outgoing.starts_with('\u{feff}') {
        outgoing.insert(0, '\u{feff}');
    }
    todo_write::atomic_write(&abs, &outgoing).map_err(ApiError::internal)?;
    st.index.write().await.upsert(&st.notes_root, &rel).map_err(ApiError::internal)?;
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let mtime = mtime_ms(&abs);
    Ok(Json(json!({
        "path": rel,
        "version": crate::index::version_of_text(mtime, &body.content),
    })))
}


/// 删除笔记：不硬删，移到项目根 `.trash/` 下（可恢复）
async fn delete_file(
    State(st): State<Arc<AppState>>,
    AxumPath(path): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let abs = st.resolve(&path).map_err(ApiError::bad_path)?;
    if !abs.is_file() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "文件不存在"));
    }
    if !path.to_ascii_lowercase().ends_with(".md") {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "只支持删除 .md"));
    }
    let rel = st.relative(&abs);
    let lock = st.lock_for(&rel).await;
    let _guard = lock.lock().await;

    let trash = st.root.join(".trash");
    std::fs::create_dir_all(&trash).map_err(ApiError::internal)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "note.md".to_string());
    let target = trash.join(format!("{stamp}-{name}"));
    std::fs::rename(&abs, &target).map_err(ApiError::internal)?;

    {
        let mut index = st.index.write().await;
        index.files.retain(|f| f.path != rel);
    }
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Ok(Json(json!({
        "path": rel,
        "trashedTo": target.display().to_string(),
    })))
}
// ---------------------------------------------------------------- 待办

async fn config(State(st): State<Arc<AppState>>) -> Json<Value> {
    let conflicts: Vec<Value> = {
        let index = st.index.read().await;
        index
            .duplicate_raw_names()
            .into_iter()
            .map(|(name, paths)| json!({ "name": name, "paths": paths }))
            .collect()
    };
    let big_files = large_assets(&st.root, 5 * 1024 * 1024);
    Json(json!({
        "links": st.links,
        "duplicateNames": conflicts,
        "largeAssets": big_files,
        "syncDrive": detect_sync_drive(&st.root),
        "noToken": st.no_token,
        "commitDebounce": st.commit_debounce,
        "commitForce": st.commit_force,
        "root": st.root.display().to_string(),
    }))
}

/// 找出 `assets/` 下超过阈值的文件（PRD 6.4：大文件进 Git 会持续撑大仓库）
fn large_assets(root: &std::path::Path, threshold: u64) -> Vec<Value> {
    let mut out = Vec::new();
    let assets = root.join("assets");
    if !assets.is_dir() {
        return out;
    }
    let mut stack = vec![assets];
    let mut visited = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > 5000 {
                return out;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.len() > threshold {
                let rel = path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                out.push(json!({
                    "path": rel,
                    "sizeMb": (meta.len() as f64 / 1024.0 / 1024.0 * 10.0).round() / 10.0,
                }));
            }
        }
    }
    out.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    out
}
async fn revision(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "revision": st.revision.load(std::sync::atomic::Ordering::SeqCst) }))
}
#[derive(Deserialize)]
struct TodoParams {
    /// 只返回带该标签（含子标签）的待办
    #[serde(default)]
    tag: Option<String>,
    /// open / done / all（默认 open）
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

async fn list_todos(
    State(st): State<Arc<AppState>>,
    Query(p): Query<TodoParams>,
) -> Json<Value> {
    let index = st.index.read().await;
    let mut rows = match p.status.as_deref() {
        Some("done") => index.all_todos().into_iter().filter(|r| r.done).collect::<Vec<_>>(),
        Some("all") => index.all_todos(),
        _ => index.open_todos(),
    };

    if let Some(tag) = p
        .tag
        .as_deref()
        .map(|t| t.trim().trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
    {
        rows.retain(|row| {
            row.tags
                .iter()
                .any(|t| t == &tag || t.starts_with(&format!("{tag}/")))
        });
    }

    let total = rows.len();
    let offset = p.offset.unwrap_or(0).min(total);
    let limit = p.limit.unwrap_or(5000).min(50_000);
    let items: Vec<_> = rows.into_iter().skip(offset).take(limit).collect();

    Json(json!({
        "total": total,
        "offset": offset,
        "returned": items.len(),
        "items": items,
    }))
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
    ensure_markdown(&body.path, "修改")?;
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
    // 按行切分并保留每行自己的行尾，避免混合 EOL 文件被整篇改写
    let mut segments: Vec<String> = content.split_inclusive('\n').map(|s| s.to_string()).collect();
    let ends_with_newline = content.ends_with('\n');

    let Some(segment) = segments.get(located).cloned() else {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "todo_relocate_failed", "行号越界"));
    };
    let segment = &segment;
    let (body_text, terminator) = if let Some(stripped) = segment.strip_suffix("\r\n") {
        (stripped.to_string(), "\r\n")
    } else if let Some(stripped) = segment.strip_suffix('\n') {
        (stripped.to_string(), "\n")
    } else {
        (segment.clone(), "")
    };
    segments[located] = format!("{}{}", todo_write::toggle_line(&body_text, body.done, &now), terminator);
    let mut updated = segments.concat();
    if ends_with_newline && !updated.ends_with('\n') {
        updated.push('\n');
    }

    todo_write::atomic_write(&abs, &updated).map_err(ApiError::internal)?;
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    st.index.write().await.upsert(&st.notes_root, &rel).map_err(ApiError::internal)?;

    let mtime = mtime_ms(&abs);
    Ok(Json(json!({
        "path": rel,
        "line": located + 1,
        "version": crate::index::version_of_text(mtime, &updated),
    })))
}

// ---------------------------------------------------------------- 新建笔记

#[derive(Deserialize)]
struct NewNoteRequest {
    /// 不传 path 时用 title 拼 `raw/<title>.md`
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    path: Option<String>,
    /// note / diary；不传时按路径判断
    #[serde(default)]
    r#type: Option<String>,
    /// templates/ 下的模板名（不含 .md）
    #[serde(default)]
    template: Option<String>,
    /// 日记日期 YYYY-MM-DD（默认今天）
    #[serde(default)]
    date: Option<String>,
}

/// 写接口统一校验：只接受 `.md`
fn ensure_markdown(path: &str, action: &str) -> Result<(), ApiError> {
    if !path.to_ascii_lowercase().ends_with(".md") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            format!("只支持{action} .md 文件"),
        ));
    }
    Ok(())
}

fn templates_dir(st: &AppState) -> PathBuf {
    st.root.join("templates")
}

async fn list_templates(State(st): State<Arc<AppState>>) -> Json<Value> {
    let dir = templates_dir(&st);
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    Json(json!({
        "dir": dir.display().to_string(),
        "templates": names,
    }))
}

fn sanitize_title(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "未命名".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

async fn create_note(
    State(st): State<Arc<AppState>>,
    Json(body): Json<NewNoteRequest>,
) -> Result<Json<Value>, ApiError> {
    let now = chrono::Local::now();
    let datetime = now.format("%Y-%m-%d %H:%M").to_string();
    let today = now.format("%Y-%m-%d").to_string();

    let title = body.title.as_deref().map(sanitize_title).unwrap_or_default();
    let date = body
        .date
        .as_deref()
        .map(str::trim)
        .filter(|d| crate::index::is_iso_date(d))
        .map(str::to_string)
        .unwrap_or_else(|| today.clone());
    let is_diary_request = body.r#type.as_deref() == Some("diary") || body.date.is_some();
    let rel = match body.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => {
            if is_diary_request {
                format!("{}/{}/{}.md", &date[..4], &date[5..7], date)
            } else if title.is_empty() {
                return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "缺少标题"));
            } else {
                format!("raw/{title}.md")
            }
        }
    };
    if !rel.ends_with(".md") {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "只支持 .md"));
    }

    let abs = st.resolve(&rel).map_err(ApiError::bad_path)?;

    let is_diary = body.r#type.as_deref() == Some("diary")
        || body.date.is_some()
        || diary_date_from_path(&rel).is_some();
    let doc_date = diary_date_from_path(&rel).unwrap_or_else(|| date.clone());
    let name = if is_diary {
        doc_date.clone()
    } else {
        title_or_stem(&title, &rel)
    };

    // 优先用 templates/<name>.md，其次内置模板
    let custom = body
        .template
        .as_deref()
        .and_then(|n| std::fs::read_to_string(templates_dir(&st).join(format!("{n}.md"))).ok());
    let content = match custom {
        Some(tpl) => render_template_full(&tpl, &name, &doc_date, &datetime),
        None if is_diary => render_template(DAILY_TEMPLATE, &doc_date, &doc_date, &datetime),
        None => render_template(NOTE_TEMPLATE, &name, &today, &datetime),
    };

    let lock_path = st.relative(&abs);
    let lock = st.lock_for(&lock_path).await;
    let _guard = lock.lock().await;
    if abs.exists() {
        return Err(ApiError::new(StatusCode::CONFLICT, "conflict", "同名文件已存在"));
    }
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
    }
    todo_write::atomic_write(&abs, &content).map_err(ApiError::internal)?;
    st.index
        .write()
        .await
        .upsert(&st.notes_root, &lock_path)
        .map_err(ApiError::internal)?;
    st.git.notify();
    st.revision
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Ok(Json(json!({
        "path": lock_path,
        "type": if is_diary { "diary" } else { "note" },
        "content": content,
        "version": crate::index::version_of_text(mtime_ms(&abs), &content),
    })))
}

fn title_or_stem(title: &str, rel: &str) -> String {
    if !title.is_empty() {
        return title.to_string();
    }
    rel.rsplit('/')
        .next()
        .unwrap_or(rel)
        .trim_end_matches(".md")
        .to_string()
}
// ---------------------------------------------------------------- 快速添加待办

// ---------------------------------------------------------------- 定向写入

/// 三个 append 接口共用请求体。`target` 是 notes/append 的旧字段名，保留兼容。
#[derive(Deserialize)]
struct AppendRequest {
    text: String,
    /// 目标文件（相对笔记根）；缺省今日日记
    #[serde(default)]
    path: Option<String>,
    /// notes/append 的旧字段名，等价于 path
    #[serde(default)]
    target: Option<String>,
    /// 显式指向今日日记（与 path/target 互斥）
    #[serde(default)]
    today: bool,
    /// 目标标题；todo/schedule 有缺省，note 不传则为文件尾追加
    #[serde(default)]
    section: Option<String>,
    /// 显式 false 时目标文件不存在即 404（CLI 防呆）
    #[serde(default)]
    create: Option<bool>,
    /// 目标段内已有完全相同行时跳过
    #[serde(default)]
    unique: Option<bool>,
    /// 只预览不写盘
    #[serde(default)]
    dry_run: Option<bool>,
}

impl AppendRequest {
    fn rel(&self, today: &str) -> String {
        self.path
            .clone()
            .or_else(|| self.target.clone())
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| format!("{}/{}/{}.md", &today[..4], &today[5..7], today))
    }

    /// 是否显式给了 --file / target（用于与 --today 的互斥校验）。
    fn has_file(&self) -> bool {
        self.path
            .as_deref()
            .or(self.target.as_deref())
            .map(|p| !p.trim().is_empty())
            .unwrap_or(false)
    }
}

/// `--file` 短名解析：只写文件名，不写一堆前缀目录。
///
/// - 含 `/`（完整路径）：原样透传，旧调用方零影响。
/// - 短名：按文件名 stem 在索引里找（大小写不敏感、自动忽略 `.md` 后缀）
///   - 唯一命中 → 展开成索引里的完整路径
///   - 多个命中 → 报错并列出全部候选，不静默取第一个（与 PRD 3.5.4 同名冲突口径一致）
///   - 零命中 → 日期形 `YYYY-MM-DD` 映射到规范日记路径；其余自动补 `.md` 走旧 create 逻辑
fn resolve_target(index: &crate::index::Index, raw: &str) -> Result<String, ApiError> {
    resolve_target_inner(index, raw).map_err(|msg| ApiError::new(StatusCode::BAD_REQUEST, "bad_request", msg))
}

/// 纯函数部分：短名 → 完整路径（或错误文案），便于单测。
fn resolve_target_inner(index: &crate::index::Index, raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    // 完整路径（含分隔符）保持旧行为，不做短名匹配
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Ok(trimmed.to_string());
    }
    // 忽略大小写地去掉 .md 后缀，得到用于匹配的 stem。
    // 先确认 n-3 落在字符边界上：--file 以多字节字符结尾（如 "😀" 4 字节）时直接按
    // 字节切会 panic，所以加 is_char_boundary 守卫。
    let n = trimmed.len();
    let stem = if n >= 3
        && trimmed.is_char_boundary(n - 3)
        && trimmed.as_bytes()[n - 3..].eq_ignore_ascii_case(b".md")
    {
        &trimmed[..n - 3]
    } else {
        trimmed
    };
    let stem_lc = stem.to_lowercase();

    let hits: Vec<String> = index
        .files
        .iter()
        .filter(|f| f.path.ends_with(".md"))
        .filter(|f| {
            let name = f.path.rsplit('/').next().unwrap_or("");
            let name_stem = name.strip_suffix(".md").unwrap_or(name);
            name_stem.to_lowercase() == stem_lc
        })
        .map(|f| f.path.clone())
        .collect();

    match hits.len() {
        0 => {
            if crate::index::is_iso_date(stem) {
                // 日期短名按目录模型展开，即使当天文件还没建也能定位
                Ok(format!("{}/{}/{}.md", &stem[..4], &stem[5..7], stem))
            } else {
                // 旧行为：补 .md 后按 notes 根相对路径处理，交给 create 逻辑
                Ok(format!("{stem}.md"))
            }
        }
        1 => Ok(hits[0].clone()),
        _ => {
            let mut sorted = hits.clone();
            sorted.sort();
            Err(format!(
                "文件名「{stem}」对应多个文件，请改用完整路径：{}",
                sorted.join("、")
            ))
        }
    }
}

/// 标题归一化：接受 `## 待办` 完整标题行，也接受 `待办` 纯标题名。
fn normalize_section(raw: &str) -> Result<String, ApiError> {
    if raw.contains('\n') || raw.contains('\r') {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "标题不能含换行",
        ));
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "标题为空"));
    }
    let out = if trimmed.starts_with('#') {
        trimmed.to_string()
    } else {
        format!("## {trimmed}")
    };
    let level = out.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 || out.trim_start_matches('#').trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "标题格式非法",
        ));
    }
    Ok(out)
}

fn sections_for(kind: write::Kind, requested: Option<String>) -> Result<Vec<String>, ApiError> {
    if let Some(raw) = requested {
        let section = normalize_section(&raw)?;
        if kind == write::Kind::Schedule {
            // 日程解析器只认这两个标题，写进别的段会静默不可见（PRD 3.3）
            if !write::SCHEDULE_SECTIONS.contains(&section.as_str()) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "日程标题只能是 ## 日程 或 ## 今日日程",
                ));
            }
            // 显式给了白名单标题时仍按候选列表解析，避免老笔记被新建第二个日程段
            let mut sections = vec![section.clone()];
            for candidate in write::SCHEDULE_SECTIONS {
                if *candidate != section && !sections.iter().any(|s| s == *candidate) {
                    sections.push((*candidate).to_string());
                }
            }
            return Ok(sections);
        }
        return Ok(vec![section]);
    }
    Ok(kind.default_sections())
}

async fn append_impl(
    st: Arc<AppState>,
    kind: write::Kind,
    body: AppendRequest,
) -> Result<Json<Value>, ApiError> {
    let text = body.text.trim();
    if text.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "内容为空"));
    }
    if text.contains('\n') || text.contains('\r') {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "一次只能添加一条",
        ));
    }
    let line = kind.format_line(text);
    let sections = sections_for(kind, body.section.clone())?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let default_rel = format!("{}/{}/{}.md", &today[..4], &today[5..7], today);
    if body.today && body.has_file() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "--today 与 --file 不能同时使用",
        ));
    }
    let rel = if body.today {
        default_rel.clone()
    } else if body.has_file() {
        let raw = body.rel(&today);
        let index = st.index.read().await;
        resolve_target(&index, &raw)?
    } else {
        default_rel.clone()
    };
    ensure_markdown(&rel, "追加")?;

    let abs = st.resolve(&rel).map_err(ApiError::bad_path)?;
    let lock_key = st.relative(&abs);
    let lock = st.lock_for(&lock_key).await;
    let _guard = lock.lock().await;

    let exists = abs.is_file();
    // 旧前端 / 旧 CLI 不传 create，沿用「不存在就按模板新建」；CLI 显式传 false 才拒绝
    if !exists && body.create == Some(false) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("文件不存在：{rel}（需要 create 才新建）"),
        ));
    }
    let is_diary_target = rel == default_rel || diary_date_from_path(&rel).is_some();
    let content = if exists {
        std::fs::read_to_string(&abs).map_err(ApiError::internal)?
    } else if is_diary_target {
        // 日记模板的标题/日期用目标文件自身日期，而不是 today：--file 2025-01-01 这类
        // 过去日期的新建日记若用 today 会写成今天的标题/日期戳，造成内容与路径不符。
        let doc_date = diary_date_from_path(&rel).unwrap_or_else(|| today.clone());
        render_template(
            DAILY_TEMPLATE,
            &doc_date,
            &doc_date,
            &chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        )
    } else {
        String::new()
    };

    let unique = body.unique.unwrap_or(false);
    let outcome = if sections.is_empty() {
        write::append_at_end(&content, &line, unique)
    } else {
        write::insert_line(&content, &sections, &line, unique)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "bad_request", e.to_string()))?
    };

    let mut value = json!({
        "ok": true,
        "path": rel.clone(),
        "section": outcome.section.clone(),
        "line": outcome.line,
        "text": line,
    });
    if body.dry_run.unwrap_or(false) {
        value["action"] = json!("dry_run");
        value["version"] = Value::Null;
        return Ok(Json(value));
    }
    if outcome.skipped {
        value["action"] = json!("skipped");
        value["version"] = json!(crate::index::version_of_text(mtime_ms(&abs), &content));
        return Ok(Json(value));
    }

    if !exists {
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
        }
    }
    todo_write::atomic_write(&abs, &outcome.content).map_err(ApiError::internal)?;
    st.index
        .write()
        .await
        .upsert(&st.notes_root, &rel)
        .map_err(ApiError::internal)?;
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    value["action"] = json!("inserted");
    value["version"] = json!(crate::index::version_of_text(mtime_ms(&abs), &outcome.content));
    Ok(Json(value))
}

async fn append_todo(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AppendRequest>,
) -> Result<Json<Value>, ApiError> {
    append_impl(st, write::Kind::Todo, body).await
}

async fn append_schedule(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AppendRequest>,
) -> Result<Json<Value>, ApiError> {
    append_impl(st, write::Kind::Schedule, body).await
}

async fn append_note(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AppendRequest>,
) -> Result<Json<Value>, ApiError> {
    append_impl(st, write::Kind::Line, body).await
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
            "version": crate::index::version_of_text(mtime, &content),
        })));
    }

    let content = render_template(DAILY_TEMPLATE, &today, &today, &chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
    }
    todo_write::atomic_write(&abs, &content).map_err(ApiError::internal)?;
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    st.index.write().await.upsert(&st.notes_root, &rel).map_err(ApiError::internal)?;

    Ok(Json(json!({
        "path": rel,
        "created": true,
        "content": content,
        "version": crate::index::version_of_text(mtime_ms(&abs), &content),
    })))
}

async fn rebuild_index(State(st): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let warnings = st.rebuild_index().await.map_err(ApiError::internal)?;
    let files = st.index.read().await.files.len();
    Ok(Json(json!({ "rebuilt": true, "files": files, "warnings": warnings })))
}

async fn git_status(State(st): State<Arc<AppState>>) -> Json<Value> {
    let status = st.git.status().await;
    Json(serde_json::to_value(status).unwrap_or_else(|_| json!({ "available": false })))
}

// ---------------------------------------------------------------- Git 历史

#[derive(Deserialize)]
struct LogParams {
    #[serde(default)]
    limit: Option<usize>,
}

fn scoped_repo(st: &AppState) -> Option<(PathBuf, String)> {
    let repo = st.git.repo_root.clone()?;
    let rel = st
        .root
        .strip_prefix(&repo)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let rel = if rel.is_empty() { ".".to_string() } else { rel };
    Some((repo, rel))
}

async fn git_log(State(st): State<Arc<AppState>>, Query(p): Query<LogParams>) -> Result<Json<Value>, ApiError> {
    let Some((repo, rel)) = scoped_repo(&st) else {
        return Ok(Json(json!({ "available": false, "commits": [] })));
    };
    let limit = p.limit.unwrap_or(80).min(500);
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .args([
                "log",
                &format!("-n{limit}"),
                "--format=%H%x1f%cI%x1f%s",
                "--",
                &rel,
            ])
            .current_dir(&repo)
            .output()
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;

    if !output.status.success() {
        return Ok(Json(json!({
            "available": false,
            "reason": String::from_utf8_lossy(&output.stderr).trim(),
            "commits": [],
        })));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\u{1f}');
        let (Some(hash), Some(iso), Some(subject)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let time = chrono::DateTime::parse_from_rfc3339(iso)
            .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|_| iso.to_string());
        commits.push(json!({
            "hash": hash,
            "short": &hash[..hash.len().min(8)],
            "time": time,
            "subject": subject,
        }));
    }

    Ok(Json(json!({ "available": true, "commits": commits })))
}

#[derive(Deserialize)]
struct ShowParams {
    hash: String,
}

async fn git_show(State(st): State<Arc<AppState>>, Query(p): Query<ShowParams>) -> Result<Json<Value>, ApiError> {
    let Some((repo, rel)) = scoped_repo(&st) else {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "git_unavailable", "未启用 git"));
    };
    if p.hash.len() < 4 || !p.hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "非法的提交号"));
    }
    let hash = p.hash.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .args(["show", "--format=", "--patch", "--no-color", &hash, "--", &rel])
            .current_dir(&repo)
            .output()
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;

    if !output.status.success() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    Ok(Json(json!({
        "hash": p.hash,
        "patch": String::from_utf8_lossy(&output.stdout).to_string(),
    })))
}
/// 常见同步盘目录检测（PRD 6.4：同步盘内启用 Git 自动提交有风险）
fn detect_sync_drive(root: &std::path::Path) -> Option<&'static str> {
    let path = root.to_string_lossy().to_lowercase();
    let rules = [
        ("onedrive", "OneDrive"),
        ("dropbox", "Dropbox"),
        ("google drive", "Google Drive"),
        ("google 云端硬盘", "Google Drive"),
        ("坚果云", "坚果云"),
        ("nutstore", "坚果云"),
        ("baidunetdisk", "百度网盘"),
        ("baidu", "百度网盘"),
        ("icloud", "iCloud"),
        ("synology", "Synology Drive"),
    ];
    for (needle, name) in rules {
        if path.contains(needle) {
            return Some(name);
        }
    }
    None
}

#[derive(Deserialize)]
struct LinksParams {
    path: String,
}

/// 出链（PRD 7.3，P1）：某文件里出现的双链及解析结果
async fn outgoing_links(
    State(st): State<Arc<AppState>>,
    Query(p): Query<LinksParams>,
) -> Json<Value> {
    let index = st.index.read().await;
    let Some(file) = index.files.iter().find(|f| f.path == p.path) else {
        return Json(json!({ "path": p.path, "total": 0, "items": [] }));
    };
    let ambiguous: std::collections::BTreeSet<String> = index
        .duplicate_raw_names()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let dir = file.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let lines: Vec<&str> = file.content.lines().collect();
    let flags = crate::markdown::code_block_flags(&lines);
    let mut items = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if flags[idx] {
            continue;
        }
        for cap in wikilink_iter(line) {
            let resolved = resolve_wikilink(&cap, dir, &ambiguous);
            let exists = !resolved.is_empty()
                && index
                    .files
                    .iter()
                    .any(|f| f.path == format!("{resolved}.md"));
            items.push(json!({
                "target": cap,
                "resolved": if exists { json!(resolved) } else { Value::Null },
                "line": idx + 1,
            }));
        }
    }

    Json(json!({ "path": p.path, "total": items.len(), "items": items }))
}
// ---------------------------------------------------------------- 回滚 / 反向链接

#[derive(Deserialize)]
struct RestoreRequest {
    hash: String,
    path: String,
}

async fn git_restore(
    State(st): State<Arc<AppState>>,
    Json(body): Json<RestoreRequest>,
) -> Result<Json<Value>, ApiError> {
    let Some((repo, _)) = scoped_repo(&st) else {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "git_unavailable", "未启用 git"));
    };
    if body.hash.len() < 4 || !body.hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "非法的提交号"));
    }
    if !body.path.ends_with(".md") {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "只支持恢复 .md 文件"));
    }
    // 前端可能传三种路径：仓库相对（含 repo→notes 完整前缀）、项目根相对（notes/...）、
    // notes 根相对（2026/11/...）。逐个候选尝试 `git show <hash>:<repo 相对>`，第一个成功的为准。
    let path_clean = body.path.trim().trim_start_matches('/').replace('\\', "/");
    let repo_to_root = st
        .root
        .strip_prefix(&repo)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let notes_prefix = st
        .notes_root
        .strip_prefix(&st.root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let full_prefix = st
        .notes_root
        .strip_prefix(&repo)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    let mut candidates: Vec<String> = Vec::new();
    if !full_prefix.is_empty() {
        if let Some(rest) = path_clean.strip_prefix(&format!("{full_prefix}/")) {
            candidates.push(rest.to_string());
        }
    }
    if !repo_to_root.is_empty() {
        if let Some(rest) = path_clean.strip_prefix(&format!("{repo_to_root}/")) {
            candidates.push(rest.to_string());
            if !notes_prefix.is_empty() {
                if let Some(inner) = rest.strip_prefix(&format!("{notes_prefix}/")) {
                    candidates.push(inner.to_string());
                }
            }
        }
    }
    if !notes_prefix.is_empty() {
        if let Some(rest) = path_clean.strip_prefix(&format!("{notes_prefix}/")) {
            candidates.push(rest.to_string());
        }
    }
    candidates.push(path_clean.clone());

    let hash = body.hash.clone();
    let mut found: Option<(std::path::PathBuf, String)> = None;
    for cand in candidates {
        let Ok(abs) = st.resolve(&cand) else { continue };
        let Ok(rel) = abs.strip_prefix(&repo) else { continue };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let spec = format!("{hash}:{rel_str}");
        let repo_clone = repo.clone();
        let output = tokio::task::spawn_blocking(move || {
            std::process::Command::new("git")
                .args(["show", &spec])
                .current_dir(&repo_clone)
                .output()
        })
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
        if output.status.success() {
            found = Some((abs, String::from_utf8_lossy(&output.stdout).to_string()));
            break;
        }
    }
    let Some((abs, content)) = found else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "该提交里没有这个文件",
        ));
    };
    let rel_path = st.relative(&abs);
    let lock = st.lock_for(&rel_path).await;
    let _guard = lock.lock().await;
    todo_write::atomic_write(&abs, &content).map_err(ApiError::internal)?;
    st.index
        .write()
        .await
        .upsert(&st.notes_root, &rel_path)
        .map_err(ApiError::internal)?;
    st.git.notify();
    st.revision
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Ok(Json(json!({
        "path": rel_path,
        "version": crate::index::version_of_text(mtime_ms(&abs), &content),
    })))
}

#[derive(Deserialize)]
struct BacklinksParams {
    path: String,
}

/// `[[目标]]` 的解析（PRD 3.5.2）：含 `/` 以 notes 根为基准；不含 `/` 只在 raw/ 下按裸名找。
fn resolve_wikilink(target: &str, from_dir: &str, ambiguous: &std::collections::BTreeSet<String>) -> String {
    let clean = target.trim();
    let clean = clean.strip_suffix(".md").unwrap_or(clean);
    if clean.contains('/') {
        normalize_rel(clean)
    } else if ambiguous.contains(clean) {
        // 同名裸名冲突期间不解析（PRD 3.5.4）
        String::new()
    } else {
        let base = format!("raw/{clean}");
        let _ = from_dir;
        normalize_rel(&base)
    }
}

fn normalize_rel(path: &str) -> String {
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            other => segs.push(other),
        }
    }
    segs.join("/")
}

async fn backlinks(
    State(st): State<Arc<AppState>>,
    Query(p): Query<BacklinksParams>,
) -> Json<Value> {
    let target = normalize_rel(p.path.trim().trim_end_matches(".md"));
    let index = st.index.read().await;
    let ambiguous: std::collections::BTreeSet<String> = index
        .duplicate_raw_names()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let mut items = Vec::new();

    for file in &index.files {
        let dir = file.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let lines: Vec<&str> = file.content.lines().collect();
        let flags = crate::markdown::code_block_flags(&lines);
        for (idx, line) in lines.iter().enumerate() {
            if flags[idx] {
                continue;
            }
            for cap in wikilink_iter(line) {
                if resolve_wikilink(&cap, dir, &ambiguous) == target && !target.is_empty() {
                    items.push(json!({
                        "path": file.path,
                        "line": idx + 1,
                        "text": line.trim(),
                    }));
                }
            }
        }
    }

    Json(json!({ "path": target, "total": items.len(), "items": items }))
}

fn wikilink_iter(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let inner = &after[..end];
        if !inner.trim().is_empty() {
            out.push(inner.trim().to_string());
        }
        rest = &after[end + 2..];
    }
    out
}
// ---------------------------------------------------------------- todo: 迁移清单

async fn migration_todos(State(st): State<Arc<AppState>>) -> Json<Value> {
    let index = st.index.read().await;
    let mut items = Vec::new();
    for file in &index.files {
        for (line, text) in crate::markdown::find_legacy_todo(&file.content) {
            items.push(json!({
                "path": file.path,
                "line": line,
                "text": text,
            }));
        }
    }
    Json(json!({ "total": items.len(), "items": items }))
}
// ---------------------------------------------------------------- 日程

#[derive(Deserialize)]
struct ScheduleParams {
    /// today / week / month，缺省 week
    #[serde(default)]
    range: Option<String>,
}

async fn schedule_view(
    State(st): State<Arc<AppState>>,
    Query(p): Query<ScheduleParams>,
) -> Json<Value> {
    use chrono::{Datelike, Duration, Local, NaiveDate};

    let today = Local::now().date_naive();
    let range = p.range.as_deref().unwrap_or("week");

    let (from, to) = match range {
        "today" => (today, today),
        "month" => {
            let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
            let next = if today.month() == 12 {
                NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
            } else {
                NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
            };
            let last = next.and_then(|n| n.pred_opt()).unwrap_or(first);
            (first, last)
        }
        _ => {
            let offset = today.weekday().num_days_from_monday() as i64;
            let monday = today - Duration::days(offset);
            (monday, monday + Duration::days(6))
        }
    };

    let index = st.index.read().await;
    let mut days = Vec::new();
    let mut cursor = from;
    while cursor <= to {
        let date = cursor.format("%Y-%m-%d").to_string();
        let is_today = cursor == today;
        let entry = index
            .files
            .iter()
            .find(|f| f.diary_date.as_deref() == Some(date.as_str()));

        match entry {
            Some(file) => {
                let section = crate::schedule::parse(&file.content);
                let suspected = if is_today {
                    crate::schedule::suspected_outside_section(&file.content)
                } else {
                    Vec::new()
                };
                days.push(json!({
                    "date": date,
                    "weekday": weekday_cn(cursor),
                    "today": is_today,
                    "path": file.path,
                    "items": section.items,
                    "unrecognized": section.unrecognized,
                    "suspected": suspected,
                }));
            }
            None if is_today => {
                days.push(json!({
                    "date": date,
                    "weekday": weekday_cn(cursor),
                    "today": true,
                    "path": null,
                    "items": [],
                    "unrecognized": [],
                }));
            }
            None => {}
        }

        cursor += Duration::days(1);
    }

    Json(json!({
        "range": range,
        "from": from.format("%Y-%m-%d").to_string(),
        "to": to.format("%Y-%m-%d").to_string(),
        "days": days,
    }))
}

fn weekday_cn(d: chrono::NaiveDate) -> &'static str {
    use chrono::Datelike;
    match d.weekday().num_days_from_monday() {
        0 => "周一",
        1 => "周二",
        2 => "周三",
        3 => "周四",
        4 => "周五",
        5 => "周六",
        _ => "周日",
    }
}

// ---------------------------------------------------------------- 标签树

#[derive(Default)]
struct TagNode {
    notes: usize,
    todos: usize,
    children: BTreeMap<String, TagNode>,
}

impl TagNode {
    /// 一个标签贡献给它自身与全部祖先：点一级标签要包含所有子标签的内容。
    fn insert(&mut self, segments: &[&str], notes: usize, todos: usize) {
        let Some((head, rest)) = segments.split_first() else {
            return;
        };
        let node = self.children.entry((*head).to_string()).or_default();
        node.notes += notes;
        node.todos += todos;
        node.insert(rest, notes, todos);
    }

    fn children_json(&self, prefix: &str) -> Vec<Value> {
        self.children
            .iter()
            .map(|(name, node)| {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                json!({
                    "name": name,
                    "path": path,
                    "notes": node.notes,
                    "todos": node.todos,
                    "children": node.children_json(&path),
                })
            })
            .collect()
    }
}

async fn list_tags(State(st): State<Arc<AppState>>) -> Json<Value> {
    let index = st.index.read().await;
    let mut root = TagNode::default();
    for file in &index.files {
        for tag in &file.tags {
            root.insert(&tag.split('/').collect::<Vec<_>>(), 1, 0);
        }
        for todo in file.todos.iter().filter(|t| !t.done) {
            for tag in &todo.tags {
                root.insert(&tag.split('/').collect::<Vec<_>>(), 0, 1);
            }
        }
    }
    Json(json!({ "tree": root.children_json("") }))
}

// ---------------------------------------------------------------- 搜索

#[derive(Deserialize)]
struct SearchParams {
    #[serde(default)]
    q: String,
    #[serde(default)]
    tag: Option<String>,
    /// open / done / all（默认 all）
    #[serde(default)]
    status: Option<String>,
    /// all / text（text = 只要正文，排除代码块命中）
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

async fn search(State(st): State<Arc<AppState>>, Query(p): Query<SearchParams>) -> Json<Value> {
    let index = st.index.read().await;
    let needle = p.q.trim().to_lowercase();
    let tag_filter = p
        .tag
        .as_deref()
        .map(|t| t.trim().trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty());
    let limit = p.limit.unwrap_or(200).min(1000);

    let mut files: Vec<&crate::index::FileEntry> = index.files.iter().collect();
    files.sort_by(|a, b| match (&a.diary_date, &b.diary_date) {
        (Some(x), Some(y)) => y.cmp(x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.path.cmp(&b.path),
    });

    let mut items: Vec<Value> = Vec::new();
    for file in files {
        if let Some(tag) = &tag_filter {
            let hit = file
                .tags
                .iter()
                .any(|t| t == tag || t.starts_with(&format!("{tag}/")));
            if !hit {
                continue;
            }
        }

        // 只有标签没有关键词：按文件返回，摘要显示「标签所在那一行」
        if needle.is_empty() {
            let (line_no, text) = match &tag_filter {
                Some(tag) => tag_line_snippet(&file.content, tag),
                None => (1, first_line(&file.content)),
            };
            items.push(json!({
                "path": file.path,
                "line": line_no,
                "snippet": text,
                "kind": "file",
            }));
            if items.len() >= limit {
                break;
            }
            continue;
        }

        let lines: Vec<&str> = file.content.lines().collect();
        let flags = crate::markdown::code_block_flags(&lines);
        for (idx, line) in lines.iter().enumerate() {
            if p.scope.as_deref() == Some("text") && flags[idx] {
                continue;
            }
            let state = crate::markdown::todo_state(line);
            match p.status.as_deref() {
                Some("open") if state == Some(true) => continue,
                Some("done") if state != Some(true) => continue,
                _ => {}
            }

            let text = if state.is_some() {
                crate::markdown::todo_text(line)
            } else {
                (*line).to_string()
            };
            if !text.to_lowercase().contains(&needle) {
                continue;
            }
            let kind = if flags[idx] {
                "code"
            } else if state.is_some() {
                "todo"
            } else {
                "text"
            };
            let before = if idx > 0 { Some(snippet(lines[idx - 1])) } else { None };
            let after = if idx + 1 < lines.len() { Some(snippet(lines[idx + 1])) } else { None };
            items.push(json!({
                "path": file.path,
                "line": idx + 1,
                "snippet": snippet(&text),
                "before": before,
                "after": after,
                "kind": kind,
                "done": state,
            }));
            if items.len() >= limit {
                break;
            }
        }
        if items.len() >= limit {
            break;
        }
    }

    Json(json!({ "total": items.len(), "items": items }))
}

fn first_line(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let body_start = frontmatter_close(&lines).map(|i| i + 1).unwrap_or(0);
    lines[body_start..]
        .iter()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && t != "---"
        })
        .map(|l| snippet(l.trim_start_matches('#').trim()))
        .unwrap_or_default()
}

/// frontmatter 结束行（第二个 `---`）的索引；不是 frontmatter 返回 None。
fn frontmatter_close(lines: &[&str]) -> Option<usize> {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return None;
    }
    lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| l.trim() == "---")
        .map(|(i, _)| i)
}

/// 标签搜索时定位「标签所在行」：先找正文里带 `#标签` 的行，再找 frontmatter 里含该标签的行，
/// 都没有则退回正文第一行。返回 (行号从 1 开始, 摘要)。修复「摘要全是 frontmatter 的 ---」。
fn tag_line_snippet(content: &str, tag: &str) -> (usize, String) {
    let lines: Vec<&str> = content.lines().collect();
    let fm = frontmatter_close(&lines);
    let body_start = fm.map(|i| i + 1).unwrap_or(0);

    for (idx, line) in lines.iter().enumerate().skip(body_start) {
        let tags = crate::markdown::extract_tags(line);
        if tags.iter().any(|t| t == tag || t.starts_with(&format!("{tag}/"))) {
            return (idx + 1, snippet(line.trim()));
        }
    }
    if let Some(end) = fm {
        for (idx, line) in lines.iter().enumerate().take(end + 1) {
            if line.contains(tag) {
                return (idx + 1, snippet(line.trim()));
            }
        }
    }
    for (idx, line) in lines.iter().enumerate().skip(body_start) {
        if !line.trim().is_empty() {
            return (idx + 1, snippet(line.trim()));
        }
    }
    (1, first_line(content))
}

fn snippet(line: &str) -> String {
    let trimmed = line.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 160 {
        trimmed.to_string()
    } else {
        let head: String = chars.iter().take(160).collect();
        format!("{head}…")
    }
}

// ---------------------------------------------------------------- 静态资源（图片 / 附件）

#[derive(Deserialize)]
struct AssetParams {
    /// 笔记里写的引用路径，如 `../../assets/a.png` 或 `assets/a.png`
    path: String,
    /// 引用所在的文件（notes 根相对路径），用于双轨解析的第一轨
    #[serde(default)]
    from: Option<String>,
}

async fn asset(
    State(st): State<Arc<AppState>>,
    Query(p): Query<AssetParams>,
) -> Result<Response, ApiError> {
    let raw = p.path.trim();
    if raw.is_empty()
        || raw.starts_with("http://")
        || raw.starts_with("https://")
        || raw.starts_with("data:")
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_path",
            "不支持的资源路径",
        ));
    }

    // 双轨：先按文件自身相对位置，再按 notes 根
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(from) = &p.from {
        if let Ok(abs) = st.resolve(from) {
            if let Some(dir) = abs.parent() {
                candidates.push(join_normalized(dir, raw));
            }
        }
    }
    candidates.push(join_normalized(&st.assets_root, raw));
    candidates.push(join_normalized(&st.notes_root, raw));
    candidates.push(join_normalized(&st.root, raw));

    for candidate in candidates {
        let candidate = crate::state::simplify(candidate);
        let inside = candidate.starts_with(&st.root)
            || candidate.starts_with(&st.notes_root)
            || candidate.starts_with(&st.assets_root);
        if !inside || !candidate.is_file() {
            continue;
        }
        if candidate.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        return serve_bytes(&candidate);
    }

    Err(ApiError::new(StatusCode::NOT_FOUND, "not_found", "资源不存在"))
}

fn join_normalized(base: &std::path::Path, rel: &str) -> PathBuf {
    let mut out = PathBuf::from(base);
    for seg in rel.replace('\\', "/").split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn serve_bytes(path: &std::path::Path) -> Result<Response, ApiError> {
    let bytes = std::fs::read(path).map_err(ApiError::internal)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(axum::body::Body::from(bytes))
        .map_err(ApiError::internal)
}
fn mtime_ms(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod sync_tests {
    use super::detect_sync_drive;
    use std::path::Path;

    #[test]
    fn detects_common_sync_drives() {
        assert_eq!(detect_sync_drive(Path::new("C:/Users/x/OneDrive/notes")), Some("OneDrive"));
        assert_eq!(detect_sync_drive(Path::new("D:/坚果云/notes")), Some("坚果云"));
        assert_eq!(detect_sync_drive(Path::new("D:/notes")), None);
    }
}

#[cfg(test)]
mod target_tests {
    use super::resolve_target_inner;
    use crate::index::{FileEntry, Index};

    fn idx(paths: &[&str]) -> Index {
        Index {
            files: paths
                .iter()
                .map(|p| FileEntry {
                    path: (*p).to_string(),
                    mtime_ms: 0,
                    size: 0,
                    version: String::new(),
                    diary_date: None,
                    content: String::new(),
                    todos: vec![],
                    tags: vec![],
                })
                .collect(),
            warnings: vec![],
        }
    }

    fn ok(raw: &str, paths: &[&str]) -> String {
        resolve_target_inner(&idx(paths), raw).unwrap()
    }

    #[test]
    fn full_path_is_untouched() {
        assert_eq!(ok("raw/读书笔记.md", &["raw/读书笔记.md"]), "raw/读书笔记.md");
        assert_eq!(ok("2026/11/2026-11-09.md", &[]), "2026/11/2026-11-09.md");
    }

    #[test]
    fn unique_short_name_expands() {
        assert_eq!(ok("2026-11-09", &["2026/11/2026-11-09.md"]), "2026/11/2026-11-09.md");
        assert_eq!(ok("读书笔记", &["raw/读书笔记.md"]), "raw/读书笔记.md");
        assert_eq!(ok("读书笔记.md", &["raw/读书笔记.md"]), "raw/读书笔记.md");
        assert_eq!(ok("README", &["raw/README.md"]), "raw/README.md");
    }

    #[test]
    fn duplicate_short_name_is_rejected_with_candidates() {
        let err = resolve_target_inner(&idx(&["raw/A.md", "raw/sub/A.md"]), "A").unwrap_err();
        assert!(err.contains("对应多个文件"));
        assert!(err.contains("raw/A.md"));
        assert!(err.contains("raw/sub/A.md"));
    }

    #[test]
    fn zero_hit_date_maps_to_diary_path() {
        assert_eq!(ok("2026-11-09", &[]), "2026/11/2026-11-09.md");
    }

    #[test]
    fn zero_hit_plain_name_gets_md_suffix() {
        assert_eq!(ok("新笔记", &[]), "新笔记.md");
    }

    #[test]
    fn multibyte_suffix_does_not_panic() {
        // 以 4 字节字符结尾时，去 .md 后缀不能按字节硬切（曾会 panic）
        assert_eq!(ok("😀", &[]), "😀.md");
        assert_eq!(ok("a😀", &[]), "a😀.md");
        // 多字节 + 大写后缀仍能正确去后缀并命中
        assert_eq!(ok("a😀.MD", &["raw/a😀.md"]), "raw/a😀.md");
    }
}

#[cfg(test)]
mod search_tests {
    use super::{first_line, tag_line_snippet};

    #[test]
    fn first_line_skips_frontmatter() {
        let doc = "---\ntitle: x\ntags: []\n---\n\n# 真正的第一行\n";
        assert_eq!(first_line(doc), "真正的第一行");
    }

    #[test]
    fn tag_line_finds_body_line() {
        let doc = "---\ntags: [工作]\n---\n\n随便\n\n- [ ] 写周报 #工作\n";
        let (line, text) = tag_line_snippet(doc, "工作");
        assert_eq!(line, 7);
        assert!(text.contains("#工作"));
    }

    #[test]
    fn tag_line_falls_back_to_frontmatter() {
        let doc = "---\ntags: [生活]\n---\n\n正文没有标签\n";
        let (line, text) = tag_line_snippet(doc, "生活");
        assert_eq!(line, 2);
        assert!(text.contains("生活"));
    }
}
