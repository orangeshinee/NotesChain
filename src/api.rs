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
    if !path.to_ascii_lowercase().ends_with(".md") {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "只支持保存 .md 文件"));
    }
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
    let outgoing = if body.eol.as_deref() == Some("crlf") {
        body.content.replace('\n', "\r\n")
    } else {
        body.content.clone()
    };
    todo_write::atomic_write(&abs, &outgoing).map_err(ApiError::internal)?;
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    st.index.write().await.upsert(&st.notes_root, &rel).map_err(ApiError::internal)?;

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
    if !body.path.to_ascii_lowercase().ends_with(".md") {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "只支持 .md 文件"));
    }
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
    if segments.len() < located + 1 {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "todo_relocate_failed", "行号越界"));
    }
    let segment = &segments[located];
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

#[derive(Deserialize)]
struct AppendTodoRequest {
    text: String,
    /// 不传则写进今天的日记
    #[serde(default)]
    path: Option<String>,
}

fn insert_into_section_any(content: &str, headings: &[&str], line: &str) -> (String, usize) {
    for heading in headings {
        if content.lines().any(|l| l.trim() == *heading) {
            return insert_into_section(content, heading, line);
        }
    }
    insert_into_section(content, headings[0], line)
}

fn insert_into_section(content: &str, heading: &str, line: &str) -> (String, usize) {
    let lines: Vec<&str> = content.lines().collect();
    // 找 `## 待办` 段
    let mut heading_idx: Option<usize> = None;
    for (idx, text) in lines.iter().enumerate() {
        if text.trim() == heading {
            heading_idx = Some(idx);
            break;
        }
    }

    match heading_idx {
        Some(h) => {
            // 段尾：下一个一级/二级标题之前
            let mut end = lines.len();
            for (idx, text) in lines.iter().enumerate().skip(h + 1) {
                let trimmed = text.trim_start();
                if trimmed.starts_with('#') {
                    let level = trimmed.chars().take_while(|c| *c == '#').count();
                    if level <= 2 {
                        end = idx;
                        break;
                    }
                }
            }
            // 去掉段尾空行
            let mut insert_at = end;
            while insert_at > h + 1 && lines[insert_at - 1].trim().is_empty() {
                insert_at -= 1;
            }
            let mut out: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
            out.insert(insert_at, line.to_string());
            let mut text = out.join("\n");
            if content.ends_with('\n') || !content.is_empty() {
                text.push('\n');
            }
            (text, insert_at + 1)
        }
        None => {
            let mut text = content.trim_end().to_string();
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&format!("{heading}\n\n"));
            text.push_str(line);
            text.push('\n');
            let count = text.lines().count();
            (text, count)
        }
    }
}

async fn append_todo(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AppendTodoRequest>,
) -> Result<Json<Value>, ApiError> {
    let text = body.text.trim();
    if text.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "内容为空"));
    }
    if text.contains('\n') {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "一次只能添加一条"));
    }
    let text = text.trim_start_matches("- [ ]").trim_start_matches("- [x]").trim();

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let rel = body
        .path
        .clone()
        .unwrap_or_else(|| format!("{}/{}/{}.md", &today[..4], &today[5..7], today));

    let abs = st.resolve(&rel).map_err(ApiError::bad_path)?;
    let lock_key = st.relative(&abs);
    let lock = st.lock_for(&lock_key).await;
    let _guard = lock.lock().await;

    let exists = abs.is_file();
    let content = if exists {
        std::fs::read_to_string(&abs).map_err(ApiError::internal)?
    } else {
        render_template(DAILY_TEMPLATE, &today, &today, &chrono::Local::now().format("%Y-%m-%d %H:%M").to_string())
    };

    let (updated, line_no) = insert_into_section_any(&content, &["## 待办"], &format!("- [ ] {text}"));
    if !exists {
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
        }
    }
    todo_write::atomic_write(&abs, &updated).map_err(ApiError::internal)?;
    st.index
        .write()
        .await
        .upsert(&st.notes_root, &rel)
        .map_err(ApiError::internal)?;
    st.git.notify();
    st.revision
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Ok(Json(json!({
        "path": rel,
        "line": line_no,
        "version": crate::index::version_of_text(mtime_ms(&abs), &updated),
    })))
}
#[derive(Deserialize)]
struct AppendScheduleRequest {
    text: String,
}

async fn append_schedule(
    State(st): State<Arc<AppState>>,
    Json(body): Json<AppendScheduleRequest>,
) -> Result<Json<Value>, ApiError> {
    let raw = body.text.trim();
    if raw.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "内容为空"));
    }
    if raw.contains('\n') {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "一次只能添加一条"));
    }
    let line = if raw.starts_with('-') {
        raw.to_string()
    } else {
        format!("- {raw}")
    };

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let rel = format!("{}/{}/{}.md", &today[..4], &today[5..7], today);
    let abs = st.resolve(&rel).map_err(ApiError::bad_path)?;
    let lock_key = st.relative(&abs);
    let lock = st.lock_for(&lock_key).await;
    let _guard = lock.lock().await;

    let exists = abs.is_file();
    let content = if exists {
        std::fs::read_to_string(&abs).map_err(ApiError::internal)?
    } else {
        render_template(DAILY_TEMPLATE, &today, &today, &chrono::Local::now().format("%Y-%m-%d %H:%M").to_string())
    };

    let (updated, line_no) = insert_into_section_any(&content, &["## 日程", "## 今日日程"], &line);
    if !exists {
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(ApiError::internal)?;
        }
    }
    todo_write::atomic_write(&abs, &updated).map_err(ApiError::internal)?;
    st.index
        .write()
        .await
        .upsert(&st.notes_root, &rel)
        .map_err(ApiError::internal)?;
    st.git.notify();
    st.revision
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Ok(Json(json!({
        "path": rel,
        "line": line_no,
        "version": crate::index::version_of_text(mtime_ms(&abs), &updated),
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
    // 前端传的是 diff 里的仓库相对路径（可能带 notes/ 前缀），统一剥成 notes 相对
    let notes_prefix = st
        .notes_root
        .strip_prefix(&st.root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let mut rel_in_notes = body.path.trim().trim_start_matches('/').to_string();
    if !notes_prefix.is_empty() {
        if let Some(rest) = rel_in_notes.strip_prefix(&format!("{notes_prefix}/")) {
            rel_in_notes = rest.to_string();
        }
    }
    let abs = st.resolve(&rel_in_notes).map_err(ApiError::bad_path)?;
    let rel = abs
        .strip_prefix(&repo)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .map_err(|_| ApiError::bad_path("文件不在仓库内"))?;

    let hash = body.hash.clone();
    let spec = format!("{hash}:{rel}");
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("git")
            .args(["show", &spec])
            .current_dir(&repo)
            .output()
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)?;

    if !output.status.success() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "该提交里没有这个文件",
        ));
    }
    let content = String::from_utf8_lossy(&output.stdout).to_string();

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

        // 只有标签没有关键词：按文件返回
        if needle.is_empty() {
            items.push(json!({
                "path": file.path,
                "line": 1,
                "snippet": first_line(&file.content),
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
    content
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| snippet(l.trim_start_matches('#').trim()))
        .unwrap_or_default()
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
