//! NotesChain CLI：服务端 API 的终端投影（设计文档 v0.1）。
//!
//! 原则：同一个二进制、只走 HTTP、绝不直接读写笔记文件；
//! today 的待办带稳定编号（未过滤列表中的全局序号），供 `todo done` 引用。

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use serde_json::Value;

use crate::token_file;

/// 退出码（设计文档 §4.5）：成功 0、服务端未启动 2、鉴权失败 3、
/// 服务端业务错误 4、其他错误 1。
pub const EXIT_OK: i32 = 0;
pub const EXIT_OTHER: i32 = 1;
pub const EXIT_NOT_RUNNING: i32 = 2;
pub const EXIT_AUTH: i32 = 3;
pub const EXIT_BUSINESS: i32 = 4;

#[derive(Subcommand, Debug)]
pub enum Command {
    /// 今日日程 + 全部未完成待办（一屏看完）
    Today,
    /// 待办：新增 / 列出 / 勾选完成
    Todo {
        #[command(subcommand)]
        cmd: TodoCmd,
    },
    /// 全文搜索（grep 式输出：路径:行号:内容）
    Search {
        /// 关键词（子串，英文不分大小写）
        query: String,
    },
    /// 查看日程（today / week / month，默认 today）；--add 追加一条
    Schedule {
        /// 范围：today | week | month
        #[arg(value_parser = ["today", "week", "month"])]
        range: Option<String>,
        /// 追加一条日程（给了它就不是查询）
        #[arg(long, value_name = "TEXT")]
        add: Option<String>,
        /// 目标文件：可只写文件名（如 2026-11-09 / 读书笔记），重名会提示；缺省今日日记
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// 显式指向今日日记（与 --file 互斥）
        #[arg(long, conflicts_with = "file")]
        today: bool,
        /// 目标标题；缺省 ## 日程（兼容 ## 今日日程）
        #[arg(long, value_name = "HEADING")]
        section: Option<String>,
        /// 目标文件不存在时允许新建
        #[arg(long)]
        create: bool,
        /// 目标段内已有完全相同行时跳过
        #[arg(long)]
        unique: bool,
        /// 只预览，零写入
        #[arg(long)]
        dry_run: bool,
    },
    /// 速记：往今日笔记追加一行自由文本
    Note {
        #[command(subcommand)]
        cmd: NoteCmd,
    },
    /// Git 状态（只读）
    Git {
        #[command(subcommand)]
        cmd: GitCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum TodoCmd {
    /// 追加一条待办（可指定文件与标题）
    Add {
        /// 待办文本（行内 #标签/子标签、!p0–!p9 由服务端解析）
        text: String,
        /// 目标文件：可只写文件名（如 2026-11-09 / 读书笔记），重名会提示；缺省今日日记
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// 显式指向今日日记（与 --file 互斥）
        #[arg(long, conflicts_with = "file")]
        today: bool,
        /// 目标标题；缺省 ## 待办
        #[arg(long, value_name = "HEADING")]
        section: Option<String>,
        /// 目标文件不存在时允许新建
        #[arg(long)]
        create: bool,
        /// 目标段内已有完全相同行时跳过
        #[arg(long)]
        unique: bool,
        /// 只预览，零写入
        #[arg(long)]
        dry_run: bool,
    },
    /// 列出未完成待办（编号为未过滤列表中的全局序号）
    List {
        /// 按多级标签过滤，如 工作/供应链
        #[arg(long)]
        tag: Option<String>,
    },
    /// 按编号勾选完成（编号来自 today / todo list 的输出）
    Done {
        /// 待办编号（1 起）
        number: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum NoteCmd {
    /// 追加一行自由文本（不传 --section 时追加到文件尾）
    Add {
        /// 文本（单行）
        text: String,
        /// 目标文件：可只写文件名（如 2026-11-09 / 读书笔记），重名会提示；缺省今日日记
        #[arg(long, value_name = "PATH")]
        file: Option<String>,
        /// 显式指向今日日记（与 --file 互斥）
        #[arg(long, conflicts_with = "file")]
        today: bool,
        /// 目标标题；不传则追加到文件尾（旧行为）
        #[arg(long, value_name = "HEADING")]
        section: Option<String>,
        /// 目标文件不存在时允许新建
        #[arg(long)]
        create: bool,
        /// 目标段内已有完全相同行时跳过
        #[arg(long)]
        unique: bool,
        /// 只预览，零写入
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum GitCmd {
    /// 查看上次自动提交时间与仓库状态（只读）
    Status,
}

/// CLI 运行入口：返回进程退出码。
pub fn run(
    dir: Option<PathBuf>,
    port: u16,
    json: bool,
    no_token: bool,
    command: Command,
) -> i32 {
    match run_inner(dir, port, json, no_token, command) {
        Ok(()) => EXIT_OK,
        Err(fail) => {
            let _ = writeln!(std::io::stderr(), "错误：{}", fail.message);
            fail.code
        }
    }
}

#[derive(Debug)]
struct CliFail {
    code: i32,
    message: String,
}

impl CliFail {
    fn new(code: i32, message: impl Into<String>) -> Self {
        CliFail {
            code,
            message: message.into(),
        }
    }
}

fn run_inner(
    dir: Option<PathBuf>,
    port: u16,
    json: bool,
    no_token: bool,
    command: Command,
) -> Result<(), CliFail> {
    let token = resolve_auth(no_token, port)?;
    let client = Client::new(port, token, json);

    // --dir 给出时校验服务端服务的确实是这个目录，避免误操作多实例中的另一个。
    if let Some(dir) = dir {
        client.verify_dir(&dir)?;
    }

    match command {
        Command::Today => cmd_today(&client),
        Command::Todo { cmd } => match cmd {
            TodoCmd::Add { text, file, today, section, create, unique, dry_run } => cmd_todo_add(
                &client,
                AppendArgs { text: &text, file: file.as_deref(), today, section: section.as_deref(), create, unique, dry_run },
            ),
            TodoCmd::List { tag } => cmd_todo_list(&client, tag.as_deref()),
            TodoCmd::Done { number } => cmd_todo_done(&client, number),
        },
        Command::Search { query } => cmd_search(&client, &query),
        Command::Schedule { range, add, file, today, section, create, unique, dry_run } => match add {
            Some(text) => cmd_schedule_add(
                &client,
                AppendArgs { text: &text, file: file.as_deref(), today, section: section.as_deref(), create, unique, dry_run },
            ),
            None => cmd_schedule(&client, range.as_deref().unwrap_or("today")),
        },
        Command::Note { cmd } => match cmd {
            NoteCmd::Add { text, file, today, section, create, unique, dry_run } => cmd_note_add(
                &client,
                AppendArgs { text: &text, file: file.as_deref(), today, section: section.as_deref(), create, unique, dry_run },
            ),
        },
        Command::Git { cmd } => match cmd {
            GitCmd::Status => cmd_git_status(&client),
        },
    }
}

// ---------------------------------------------------------------- 鉴权

/// 鉴权优先级（设计文档 §5.1）：NC_TOKEN 环境变量 > 本地 token 文件 > --no-token。
/// 纯函数部分便于单测；真实环境变量 / 文件读取在外层。
fn resolve_auth_with(no_token: bool, port: u16, env_token: Option<&str>, file: Result<token_file::TokenFile, String>) -> Result<Option<String>, CliFail> {
    if no_token {
        return Ok(None);
    }
    if let Some(t) = env_token.map(str::trim).filter(|t| !t.is_empty()) {
        return Ok(Some(t.to_string()));
    }
    match file {
        Ok(tf) => {
            if tf.port != port {
                return Err(CliFail::new(
                    EXIT_OTHER,
                    format!(
                        "token 文件记录的端口是 {0}，与当前目标端口 {1} 不一致（可能有多个服务端实例在跑）；请用 --port {0}，或确认目标服务端的端口",
                        tf.port, port
                    ),
                ));
            }
            Ok(Some(tf.token))
        }
        Err(e) => Err(CliFail::new(
            EXIT_OTHER,
            format!(
                "未找到可用 token：{e}。若服务端以 --no-token 启动，请加 --no-token；否则请确认服务端已启动"
            ),
        )),
    }
}

fn resolve_auth(no_token: bool, port: u16) -> Result<Option<String>, CliFail> {
    let env_token = std::env::var("NC_TOKEN").ok();
    resolve_auth_with(no_token, port, env_token.as_deref(), token_file::read_token_file())
}

// ---------------------------------------------------------------- HTTP 层

struct Client {
    agent: ureq::Agent,
    base: String,
    token: Option<String>,
    json: bool,
}

impl Client {
    fn new(port: u16, token: Option<String>, json: bool) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        Client {
            agent,
            base: format!("http://127.0.0.1:{port}"),
            token,
            json,
        }
    }

    /// 发请求，返回响应体原文。错误按退出码约定翻译。
    fn request(&self, method: &str, path: &str, query: &[(&str, &str)], body: Option<&Value>) -> Result<String, CliFail> {
        let url = format!("{}{}", self.base, path);
        let mut req = match method {
            "GET" => self.agent.get(&url),
            "POST" => self.agent.post(&url),
            "PATCH" => self.agent.patch(&url),
            other => {
                return Err(CliFail::new(
                    EXIT_OTHER,
                    format!("内部错误：未支持的 HTTP 方法 {other}"),
                ))
            }
        };
        for (k, v) in query {
            req = req.query(k, v);
        }
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let resp = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match resp {
            Ok(r) => r.into_string().map_err(|e| {
                CliFail::new(EXIT_OTHER, format!("读取服务端响应失败：{e}"))
            }),
            Err(ureq::Error::Status(401, r)) => {
                let msg = error_message(&r.into_string().unwrap_or_default())
                    .unwrap_or_else(|| "token 缺失或已失效，服务端可能已重启".to_string());
                Err(CliFail::new(
                    EXIT_AUTH,
                    format!("鉴权失败：{msg}（服务端重启后 token 会变，CLI 靠 token 文件自动跟随；仍失败请重启服务端）"),
                ))
            }
            Err(ureq::Error::Status(_, r)) => {
                let raw = r.into_string().unwrap_or_default();
                let msg = error_message(&raw).unwrap_or(raw);
                Err(CliFail::new(EXIT_BUSINESS, format!("服务端返回错误：{msg}")))
            }
            Err(ureq::Error::Transport(e)) => Err(CliFail::new(
                EXIT_NOT_RUNNING,
                format!("无法连接服务端 {}（{}），请先启动服务", self.base, e),
            )),
        }
    }

    fn get_json(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, CliFail> {
        let raw = self.request("GET", path, query, None)?;
        parse_json(&raw)
    }

    fn get_raw(&self, path: &str, query: &[(&str, &str)]) -> Result<String, CliFail> {
        self.request("GET", path, query, None)
    }

    /// --dir 校验：服务端 /api/config 返回的 root 必须与 --dir 一致。
    fn verify_dir(&self, dir: &std::path::Path) -> Result<(), CliFail> {
        let canon = dir.canonicalize().map_err(|e| {
            CliFail::new(EXIT_OTHER, format!("笔记目录不可访问 {}：{e}", dir.display()))
        })?;
        let cfg = self.get_json("/api/config", &[])?;
        let root = cfg["root"].as_str().unwrap_or("");
        let server_root = PathBuf::from(root);
        // 服务端启动时已 canonicalize；这里同样规范化后再比较。
        let server_canon = server_root.canonicalize().unwrap_or(server_root);
        if server_canon != canon {
            return Err(CliFail::new(
                EXIT_OTHER,
                format!(
                    "服务端笔记目录是 {}，与 --dir {} 不一致，拒绝操作（可能连到了另一个服务端实例）",
                    server_canon.display(),
                    canon.display()
                ),
            ));
        }
        Ok(())
    }
}

/// 从服务端错误包络 { "error": { "code", "message" } } 里取 message。
fn error_message(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    v.get("error")?.get("message")?.as_str().map(|s| s.to_string())
}

/// 把服务端返回体解析为 JSON；失败统一映射为「其他错误」。
fn parse_json(raw: &str) -> Result<Value, CliFail> {
    serde_json::from_str(raw)
        .map_err(|e| CliFail::new(EXIT_OTHER, format!("服务端响应不是合法 JSON：{e}")))
}

/// json 模式下原样打印服务端返回体；返回 true 表示已输出（调用方应提前返回）。
fn print_raw_if_json(client: &Client, raw: &str) -> bool {
    if client.json {
        println!("{raw}");
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------- 待办列表与编号

/// GET /api/todos（未过滤、未完成）返回的 items；编号即其在数组中的位置（1 起）。
fn fetch_open_todos(client: &Client) -> Result<Vec<Value>, CliFail> {
    let v = client.get_json("/api/todos", &[])?;
    let total = v["total"].as_u64().unwrap_or(0);
    let items = v["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if total > items.len() as u64 {
        eprintln!("提示：未完成待办共 {total} 条，仅取回前 {} 条，编号只覆盖这部分", items.len());
    }
    Ok(items)
}

fn tag_matches(item_tags: &[Value], tag: &str) -> bool {
    item_tags.iter().any(|t| {
        let t = t.as_str().unwrap_or("");
        t == tag || t.starts_with(&format!("{tag}/"))
    })
}

/// 渲染一条待办：[编号] 文本  路径:行号  #标签  !pN
fn format_todo(n: usize, item: &Value) -> String {
    let text = item["text"].as_str().unwrap_or("");
    let path = item["path"].as_str().unwrap_or("");
    let line = item["line"].as_u64().unwrap_or(0);
    let mut s = format!("[{n}] {text}  {path}:{line}");
    if let Some(tags) = item["tags"].as_array() {
        for t in tags {
            if let Some(t) = t.as_str() {
                s.push_str(&format!("  #{t}"));
            }
        }
    }
    if let Some(p) = item["priority"].as_u64() {
        s.push_str(&format!("  !p{p}"));
    }
    s
}

// ---------------------------------------------------------------- 各命令

fn cmd_today(client: &Client) -> Result<(), CliFail> {
    let schedule_raw = client.get_raw("/api/schedule", &[("range", "today")])?;
    let todos_raw = client.get_raw("/api/todos", &[])?;
    let schedule: Value = parse_json(&schedule_raw)?;
    let todos: Value = parse_json(&todos_raw)?;
    if client.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "schedule": schedule, "todos": todos }))
                .unwrap_or_default()
        );
        return Ok(());
    }
    let mut out = String::new();
    out.push_str(&format_schedule_human(&schedule));
    let items = todos["items"].as_array().cloned().unwrap_or_default();
    out.push_str(&format!("\n## 未完成待办（{}）\n", items.len()));
    if items.is_empty() {
        out.push_str("（无未完成待办）\n");
    } else {
        for (i, item) in items.iter().enumerate() {
            out.push_str(&format_todo(i + 1, item));
            out.push('\n');
        }
    }
    print!("{out}");
    Ok(())
}

/// 三个 append 子命令共用的参数（收进结构体，避免函数参数过多）。
struct AppendArgs<'a> {
    text: &'a str,
    file: Option<&'a str>,
    today: bool,
    section: Option<&'a str>,
    create: bool,
    unique: bool,
    dry_run: bool,
}

impl AppendArgs<'_> {
    fn body(&self) -> Value {
        let mut body = serde_json::json!({
            "text": self.text,
            "unique": self.unique,
            "dry_run": self.dry_run,
        });
        if self.create {
            body["create"] = serde_json::json!(true);
        }
        if self.today {
            body["today"] = serde_json::json!(true);
        }
        if let Some(f) = self.file {
            body["path"] = serde_json::json!(f);
        }
        if let Some(s) = self.section {
            body["section"] = serde_json::json!(s);
        }
        body
    }

    fn post(&self, client: &Client, path: &str) -> Result<(), CliFail> {
        let raw = client.request("POST", path, &[], Some(&self.body()))?;
        print_append_result(client, &raw)
    }
}

fn print_append_result(client: &Client, raw: &str) -> Result<(), CliFail> {
    if print_raw_if_json(client, raw) {
        return Ok(());
    }
    let v: Value = parse_json(raw)?;
    let label = match v["action"].as_str().unwrap_or("inserted") {
        "dry_run" => "预览",
        "skipped" => "已存在",
        _ => "已写入",
    };
    println!(
        "[{label}] {}:{}  {}",
        v["path"].as_str().unwrap_or(""),
        v["line"].as_u64().unwrap_or(0),
        v["section"].as_str().unwrap_or("")
    );
    Ok(())
}

fn cmd_todo_add(client: &Client, a: AppendArgs<'_>) -> Result<(), CliFail> {
    a.post(client, "/api/todos/append")
}

fn cmd_schedule_add(client: &Client, a: AppendArgs<'_>) -> Result<(), CliFail> {
    a.post(client, "/api/schedule/append")
}

fn cmd_todo_list(client: &Client, tag: Option<&str>) -> Result<(), CliFail> {
    // 取未过滤列表再在本地按标签过滤：这样 --tag 过滤后显示的仍是全局编号，
    // 与 todo done 的编号口径一致，不会因先过滤再编号而勾错条目。
    let items = fetch_open_todos(client)?;
    let shown: Vec<(usize, &Value)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| match tag {
            Some(t) => item["tags"]
                .as_array()
                .map(|tags| tag_matches(tags, t.trim().trim_start_matches('#')))
                .unwrap_or(false),
            None => true,
        })
        .map(|(i, item)| (i + 1, item))
        .collect();
    if client.json {
        let filtered: Vec<Value> = shown
            .iter()
            .map(|(n, item)| {
                let mut v = (*item).clone();
                v["number"] = serde_json::json!(n);
                v
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "total": items.len(), "returned": filtered.len(), "items": filtered
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    if shown.is_empty() {
        println!("（无未完成待办）");
        return Ok(());
    }
    for (n, item) in shown {
        println!("{}", format_todo(n, item));
    }
    Ok(())
}

fn cmd_todo_done(client: &Client, number: usize) -> Result<(), CliFail> {
    let items = fetch_open_todos(client)?;
    let item = items.get(number.wrapping_sub(1)).ok_or_else(|| {
        CliFail::new(
            EXIT_OTHER,
            format!("编号 {number} 超出范围：当前共 {} 条未完成待办", items.len()),
        )
    })?;
    let raw = client.request(
        "PATCH",
        "/api/todos",
        &[],
        Some(&serde_json::json!({
            "path": item["path"].as_str().unwrap_or(""),
            "line": item["line"].as_u64().unwrap_or(0),
            "raw_line": item["raw_line"].as_str().unwrap_or(""),
            "done": true,
        })),
    )?;
    if print_raw_if_json(client, &raw) {
        return Ok(());
    }
    println!(
        "已完成：{}（完成时间已回写）",
        item["text"].as_str().unwrap_or("")
    );
    Ok(())
}

fn cmd_search(client: &Client, query: &str) -> Result<(), CliFail> {
    let raw = client.get_raw("/api/search", &[("q", query)])?;
    if print_raw_if_json(client, &raw) {
        return Ok(());
    }
    let v: Value = parse_json(&raw)?;
    let items = v["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        println!("（无匹配结果）");
        return Ok(());
    }
    for item in items {
        println!(
            "{}:{}:{}",
            item["path"].as_str().unwrap_or(""),
            item["line"].as_u64().unwrap_or(0),
            item["snippet"].as_str().unwrap_or("")
        );
    }
    Ok(())
}

/// 日程人类可读输出（schedule 与 today 共用）。
fn format_schedule_human(schedule: &Value) -> String {
    let mut out = String::new();
    let days = schedule["days"].as_array().cloned().unwrap_or_default();
    for day in days {
        let date = day["date"].as_str().unwrap_or("");
        let weekday = day["weekday"].as_str().unwrap_or("");
        out.push_str(&format!("## {date} {weekday}\n"));
        let items = day["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            out.push_str("（无日程）\n");
        }
        for item in items {
            let start = item["start"].as_str().unwrap_or("");
            let end = item["end"].as_str().unwrap_or("");
            let title = item["title"].as_str().unwrap_or("");
            let mut line = if end.is_empty() {
                format!("{start} {title}")
            } else {
                format!("{start}-{end} {title}")
            };
            if let Some(place) = item["place"].as_str() {
                if !place.is_empty() {
                    line.push_str(&format!(" @{place}"));
                }
            }
            if let Some(tags) = item["tags"].as_array() {
                for t in tags {
                    if let Some(t) = t.as_str() {
                        line.push_str(&format!(" #{t}"));
                    }
                }
            }
            out.push_str(&line);
            out.push('\n');
        }
        if let Some(unrec) = day["unrecognized"].as_array() {
            for u in unrec {
                if let Some(u) = u.as_str() {
                    out.push_str(&format!("  未识别：{u}\n"));
                }
            }
        }
    }
    out
}

fn cmd_schedule(client: &Client, range: &str) -> Result<(), CliFail> {
    let raw = client.get_raw("/api/schedule", &[("range", range)])?;
    if print_raw_if_json(client, &raw) {
        return Ok(());
    }
    let v: Value = parse_json(&raw)?;
    print!("{}", format_schedule_human(&v));
    Ok(())
}

fn cmd_note_add(client: &Client, a: AppendArgs<'_>) -> Result<(), CliFail> {
    a.post(client, "/api/notes/append")
}

fn cmd_git_status(client: &Client) -> Result<(), CliFail> {
    let raw = client.get_raw("/api/git/status", &[])?;
    if print_raw_if_json(client, &raw) {
        return Ok(());
    }
    let v: Value = parse_json(&raw)?;
    let available = v["available"].as_bool().unwrap_or(false);
    if !available {
        println!("Git：不可用");
        if v["disabled_by_user"].as_bool().unwrap_or(false) {
            println!("原因：已用 --no-git 关闭 Git 自动提交");
        }
        if let Some(e) = v["last_error"].as_str() {
            if !e.is_empty() {
                println!("原因：{e}");
            }
        }
        return Ok(());
    }
    println!("Git：可用");
    if let Some(repo) = v["repo_root"].as_str() {
        println!("仓库：{repo}");
    }
    match v["last_commit"].as_str() {
        Some(t) if !t.is_empty() => println!("上次提交：{t}"),
        _ => println!("上次提交：无"),
    }
    println!("待提交文件：{}", v["pending_files"].as_u64().unwrap_or(0));
    if let Some(note) = v["note"].as_str() {
        println!("{note}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_priority_env_over_file() {
        let file = Ok(token_file::TokenFile {
            token: "file-token".into(),
            port: 8090,
            started_at: 1,
        });
        // 环境变量优先于文件
        let r = resolve_auth_with(false, 8090, Some("env-token"), file.clone()).unwrap();
        assert_eq!(r.as_deref(), Some("env-token"));
        // --no-token 时一律不带 token
        assert_eq!(resolve_auth_with(true, 8090, Some("env-token"), file.clone()).unwrap(), None);
        // 无环境变量时用文件
        assert_eq!(resolve_auth_with(false, 8090, None, file).unwrap().as_deref(), Some("file-token"));
    }

    #[test]
    fn auth_rejects_port_mismatch() {
        let file = Ok(token_file::TokenFile {
            token: "t".into(),
            port: 8099,
            started_at: 1,
        });
        let r = resolve_auth_with(false, 8090, None, file);
        assert!(r.is_err());
        assert!(r.unwrap_err().message.contains("8099"));
    }

    #[test]
    fn auth_missing_token_file_explains() {
        let r = resolve_auth_with(false, 8090, None, Err("文件不存在".into()));
        assert!(r.is_err());
        assert!(r.unwrap_err().message.contains("--no-token"));
    }

    #[test]
    fn format_todo_includes_number_tags_priority() {
        let item = serde_json::json!({
            "text": "回复供应商邮件", "path": "2026/11/2026-11-04.md", "line": 5,
            "tags": ["工作/供应链"], "priority": 1
        });
        assert_eq!(
            format_todo(2, &item),
            "[2] 回复供应商邮件  2026/11/2026-11-04.md:5  #工作/供应链  !p1"
        );
        let plain = serde_json::json!({
            "text": "买牛奶", "path": "2026/11/2026-11-04.md", "line": 9,
            "tags": [], "priority": serde_json::Value::Null
        });
        assert_eq!(format_todo(1, &plain), "[1] 买牛奶  2026/11/2026-11-04.md:9");
    }

    #[test]
    fn tag_matches_prefix_rule() {
        let tags = vec![
            serde_json::json!("工作/供应链"),
            serde_json::json!("生活"),
        ];
        assert!(tag_matches(&tags, "工作"));
        assert!(tag_matches(&tags, "工作/供应链"));
        assert!(!tag_matches(&tags, "工作/采购"));
        assert!(tag_matches(&tags, "生活"));
    }

    #[test]
    fn error_message_extracted_from_envelope() {
        let raw = r#"{"error":{"code":"conflict","message":"版本不一致"}}"#;
        assert_eq!(error_message(raw).as_deref(), Some("版本不一致"));
        assert_eq!(error_message("not json"), None);
    }

    #[test]
    fn schedule_human_format() {
        let v = serde_json::json!({
            "days": [
                { "date": "2026-11-05", "weekday": "周四", "items": [
                    { "start": "15:00", "end": "17:00", "title": "供应链周会", "tags": [], "place": "线上" }
                ], "unrecognized": [] },
                { "date": "2026-11-06", "weekday": "周五", "items": [], "unrecognized": [] }
            ]
        });
        let out = format_schedule_human(&v);
        assert!(out.contains("## 2026-11-05 周四"));
        assert!(out.contains("15:00-17:00 供应链周会 @线上"));
        assert!(out.contains("## 2026-11-06 周五"));
        assert!(out.contains("（无日程）"));
    }
}
