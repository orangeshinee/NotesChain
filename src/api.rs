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
const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_JS: &str = include_str!("../web/app.js");
const STYLE_CSS: &str = include_str!("../web/style.css");
const EDITOR_JS: &str = include_str!("../web/dist/editor.js");

pub fn router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/api/files", get(list_files))
        .route("/api/files/*path", get(read_file).put(save_file))
        .route("/api/todos", get(list_todos).patch(patch_todo))
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
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/editor.js", get(editor_js))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth));

    Router::new()
        .route("/", get(root))
        .merge(protected)
        .with_state(state)
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn style_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLE_CSS)
}

async fn editor_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
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
    let abs = st.resolve(&path).map_err(ApiError::bad_path)?;
    let rel = st.relative(&abs);
    let lock = st.lock_for(&rel).await;
    let _guard = lock.lock().await;

    let current = std::fs::read_to_string(&abs).unwrap_or_default();
    let current_version = crate::index::version_of(mtime_ms(&abs), &current);
    if current_version != body.version && !q.force.unwrap_or(false) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            "文件已被外部修改，请重新加载或强制覆盖",
        ));
    }

    todo_write::atomic_write(&abs, &body.content).map_err(ApiError::internal)?;
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    st.index.write().await.upsert(&st.root, &rel).map_err(ApiError::internal)?;

    let mtime = mtime_ms(&abs);
    Ok(Json(json!({
        "path": rel,
        "version": crate::index::version_of(mtime, &body.content),
    })))
}

// ---------------------------------------------------------------- 待办

async fn config(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "links": st.links }))
}
async fn revision(State(st): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "revision": st.revision.load(std::sync::atomic::Ordering::SeqCst) }))
}
#[derive(Deserialize)]
struct TodoParams {
    /// 只返回带该标签（含子标签）的待办
    #[serde(default)]
    tag: Option<String>,
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
    let mut rows = index.open_todos();

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
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
    st.git.notify();
    st.revision.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
                days.push(json!({
                    "date": date,
                    "weekday": weekday_cn(cursor),
                    "today": is_today,
                    "path": file.path,
                    "items": section.items,
                    "unrecognized": section.unrecognized,
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
            items.push(json!({
                "path": file.path,
                "line": idx + 1,
                "snippet": snippet(&text),
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
    candidates.push(join_normalized(&st.root, raw));

    for candidate in candidates {
        let candidate = crate::state::simplify(candidate);
        if !candidate.starts_with(&st.root) || !candidate.is_file() {
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
