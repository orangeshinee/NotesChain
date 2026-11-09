//! CLI 子命令：定向写入、实例登记表、最小 HTTP 客户端。
//!
//! 设计见 docs/plan-2026-11-09-cli-write.md。要点：
//! - 登记表放用户级目录，token 绝不写进笔记目录（会被 Git 自动提交收走）。
//! - CLI 不自动拉起 server，也不自动重试，避免重复落盘。
//! - 只连 127.0.0.1，无 TLS，不引第三方 HTTP 依赖，守住单文件体积。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args as ClapArgs, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::write::Kind;

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_FOUND: i32 = 3;
pub const EXIT_LOCATE: i32 = 4;
pub const EXIT_CONFLICT: i32 = 5;

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 定向写入一行到指定文件与标题下
    Add {
        #[command(subcommand)]
        kind: AddKind,
    },
}

#[derive(Subcommand, Debug)]
pub enum AddKind {
    /// 追加一条待办 `- [ ] ...`
    Todo(AddArgs),
    /// 追加一条日程 `- HH:mm ...`
    Schedule(AddArgs),
    /// 追加一行原文（不改格式）
    Line(AddArgs),
}

#[derive(ClapArgs, Debug, Clone)]
pub struct AddArgs {
    /// 内容（单行）
    #[arg(long)]
    pub text: String,
    /// 目标文件，相对 notes 根；缺省今天的日记
    #[arg(long, value_name = "PATH")]
    pub file: Option<String>,
    /// 目标标题（`## 待办` 或 `待办`）；add line 必填
    #[arg(long, value_name = "HEADING")]
    pub section: Option<String>,
    /// 笔记根目录，用于多实例消歧
    #[arg(long, value_name = "DIR")]
    pub dir: Option<String>,
    /// 服务端口，用于多实例消歧
    #[arg(long)]
    pub port: Option<u16>,
    /// 覆盖实例登记表里的 token（调试用）
    #[arg(long)]
    pub token: Option<String>,
    /// 目标文件不存在时允许新建
    #[arg(long)]
    pub create: bool,
    /// 目标段内已有完全相同行时跳过
    #[arg(long)]
    pub unique: bool,
    /// 只预览，零写入
    #[arg(long)]
    pub dry_run: bool,
    /// 结构化输出（供脚本 / cron 解析）
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------- 实例登记表

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub root: String,
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: String,
}

/// 用户级固定位置。CLI 从这里读，不需要先知道笔记根目录。
fn registry_path() -> PathBuf {
    #[cfg(windows)]
    let path = {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("NotesChain").join("instances.json")
    };
    #[cfg(not(windows))]
    let path = {
        if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
            PathBuf::from(xdg).join("noteschain").join("instances.json")
        } else {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".cache").join("noteschain").join("instances.json")
        }
    };
    path
}

fn load_registry() -> Vec<Instance> {
    let Ok(text) = std::fs::read_to_string(registry_path()) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_registry(list: &[Instance]) -> Result<(), String> {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        let mut file = opts.open(&path).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn register(instance: &Instance) {
    let mut list = load_registry();
    list.retain(|i| !(i.root == instance.root && i.port == instance.port));
    list.push(instance.clone());
    if let Err(err) = save_registry(&list) {
        eprintln!("[cli] 登记实例失败: {err}");
    }
}

pub fn unregister(instance: &Instance) {
    let mut list = load_registry();
    list.retain(|i| {
        !(i.root == instance.root && i.port == instance.port && i.pid == instance.pid)
    });
    if let Err(err) = save_registry(&list) {
        eprintln!("[cli] 注销实例失败: {err}");
    }
}

fn alive(instance: &Instance) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], instance.port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// 只保留活实例；陈旧条目（server 崩溃残留）不自动清理，靠端口探测过滤。
fn select(args: &AddArgs) -> Result<Instance, (i32, String)> {
    let mut list: Vec<Instance> = load_registry().into_iter().filter(alive).collect();
    if let Some(dir) = &args.dir {
        let canon = std::fs::canonicalize(dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| dir.clone());
        list.retain(|i| i.root == canon || i.root == *dir);
    }
    if let Some(port) = args.port {
        list.retain(|i| i.port == port);
    }
    if list.is_empty() {
        return Err((
            EXIT_LOCATE,
            "找不到运行中的 NotesChain 实例。请先启动服务；多实例时用 --dir / --port 消歧。"
                .to_string(),
        ));
    }
    if list.len() > 1 {
        let desc = list
            .iter()
            .map(|i| format!("{}:{} (pid {})", i.root, i.port, i.pid))
            .collect::<Vec<_>>()
            .join(", ");
        return Err((
            EXIT_LOCATE,
            format!("有多个实例：{desc}。用 --dir 或 --port 指定。"),
        ));
    }
    Ok(list.remove(0))
}

// ---------------------------------------------------------------- 最小 HTTP 客户端

fn build_request(port: u16, endpoint: &str, token: &str, payload: &str) -> String {
    let cookie = if token.is_empty() {
        String::new()
    } else {
        format!("Cookie: nc_token={token}\r\n")
    };
    format!(
        "POST {endpoint} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\n\
         {cookie}\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Accept: application/json\r\n\
         \r\n\
         {payload}",
        payload.len()
    )
}

fn parse_response(buf: &[u8]) -> Result<(u16, Value), String> {
    let sep = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "HTTP 响应缺头部".to_string())?;
    let head = String::from_utf8_lossy(&buf[..sep]).to_ascii_lowercase();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "HTTP 状态行非法".to_string())?;
    let mut raw = buf[sep + 4..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        raw = decode_chunked(&raw)?;
    }
    let text = String::from_utf8_lossy(&raw).trim().to_string();
    let value: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        json!({ "error": { "code": "internal", "message": text } })
    });
    Ok((status, value))
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        if pos >= data.len() {
            return Err("chunked 数据不完整".to_string());
        }
        let rel = data[pos..]
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| "chunked 缺尺寸行".to_string())?;
        let end = pos + rel;
        let size_hex = String::from_utf8_lossy(&data[pos..end]);
        let size = usize::from_str_radix(size_hex.split(';').next().unwrap_or("0").trim(), 16)
            .map_err(|e| e.to_string())?;
        pos = end + 2;
        if size == 0 {
            break;
        }
        if pos + size > data.len() {
            return Err("chunked 数据不完整".to_string());
        }
        out.extend_from_slice(&data[pos..pos + size]);
        pos += size + 2;
    }
    Ok(out)
}

fn post_json(port: u16, endpoint: &str, token: &str, body: &Value) -> Result<(u16, Value), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|e| format!("连接 127.0.0.1:{port} 失败: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let payload = serde_json::to_string(body).map_err(|e| e.to_string())?;
    let request = build_request(port, endpoint, token, &payload);
    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    parse_response(&buf)
}

// ---------------------------------------------------------------- 执行

fn error_code(exit: i32) -> &'static str {
    match exit {
        EXIT_USAGE => "bad_request",
        EXIT_NOT_FOUND => "not_found",
        EXIT_LOCATE => "locate_failed",
        _ => "conflict",
    }
}

fn execute(kind: Kind, args: &AddArgs) -> Result<Value, (i32, String)> {
    let text = args.text.trim();
    if text.is_empty() {
        return Err((EXIT_USAGE, "内容为空（--text 必填）".to_string()));
    }
    if text.contains('\n') || text.contains('\r') {
        return Err((
            EXIT_USAGE,
            "一次只能添加一条（--text 不能含换行）".to_string(),
        ));
    }
    if kind == Kind::Line && args.section.is_none() {
        return Err((EXIT_USAGE, "add line 需要 --section".to_string()));
    }

    let instance = select(args)?;
    let token = args.token.clone().unwrap_or_else(|| instance.token.clone());
    // CLI 一律显式传 create；默认 false 即「文件不存在直接拒绝」。
    // 旧前端不传这个字段，服务端保持自动新建的旧行为。
    let mut body = json!({
        "text": text,
        "create": args.create,
        "unique": args.unique,
        "dry_run": args.dry_run,
    });
    if let Some(file) = &args.file {
        body["path"] = json!(file);
    }
    if let Some(section) = &args.section {
        body["section"] = json!(section);
    }

    let (status, value) = post_json(instance.port, kind.endpoint(), &token, &body)
        .map_err(|e| (EXIT_LOCATE, format!("请求失败：{e}")))?;
    if !(200..300).contains(&status) {
        let code = value["error"]["code"].as_str().unwrap_or("unknown");
        let message = value["error"]["message"].as_str().unwrap_or("请求失败");
        let exit = match code {
            "bad_request" => EXIT_USAGE,
            "not_found" => EXIT_NOT_FOUND,
            "unauthorized" => EXIT_LOCATE,
            _ => EXIT_CONFLICT,
        };
        return Err((exit, format!("{code}: {message}")));
    }
    Ok(value)
}

fn print_output(args: &AddArgs, value: &Value) {
    if args.json {
        println!("{}", serde_json::to_string(value).unwrap_or_default());
        return;
    }
    let action = value["action"].as_str().unwrap_or("inserted");
    let path = value["path"].as_str().unwrap_or("");
    let section = value["section"].as_str().unwrap_or("");
    let line = value["line"].as_u64().unwrap_or(0);
    let text = value["text"].as_str().unwrap_or("");
    let label = match action {
        "dry_run" => "预览",
        "skipped" => "已存在",
        _ => "已写入",
    };
    println!("[{label}] {path} → {section}:{line}  {text}");
}

pub fn run(command: Commands) -> i32 {
    let Commands::Add { kind } = command;
    let (kind, args) = match kind {
        AddKind::Todo(a) => (Kind::Todo, a),
        AddKind::Schedule(a) => (Kind::Schedule, a),
        AddKind::Line(a) => (Kind::Line, a),
    };
    match execute(kind, &args) {
        Ok(value) => {
            print_output(&args, &value);
            EXIT_OK
        }
        Err((exit, message)) => {
            if args.json {
                println!(
                    "{}",
                    json!({ "ok": false, "code": error_code(exit), "message": message })
                );
            } else {
                eprintln!("{message}");
            }
            exit
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_outside_notes_root() {
        let path = registry_path();
        assert!(path.to_string_lossy().contains("instances.json"));
        assert!(!path.to_string_lossy().contains("/notes/"));
    }

    #[test]
    fn request_carries_cookie_and_content_length() {
        let req = build_request(8090, "/api/todos/append", "abc", "{\"text\":\"x\"}");
        assert!(req.starts_with("POST /api/todos/append HTTP/1.1\r\n"));
        assert!(req.contains("Cookie: nc_token=abc\r\n"));
        assert!(req.contains("Content-Length: 12\r\n"));
        assert!(req.contains("\r\n\r\n{\"text\":\"x\"}"));
    }

    #[test]
    fn request_omits_cookie_when_no_token() {
        let req = build_request(8090, "/api/todos/append", "", "{}");
        assert!(!req.contains("Cookie"));
        assert!(req.contains("Content-Length: 2\r\n"));
    }

    #[test]
    fn parses_content_length_response() {
        let body = r#"{"ok":true,"action":"inserted"}"#;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (status, value) = parse_response(resp.as_bytes()).unwrap();
        assert_eq!(status, 200);
        assert_eq!(value["action"], "inserted");
    }

    #[test]
    fn parses_chunked_response() {
        let body = r#"{"ok":true}"#;
        let chunk = format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
        let resp = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{chunk}"
        );
        let (status, value) = parse_response(resp.as_bytes()).unwrap();
        assert_eq!(status, 200);
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn instance_roundtrip_json() {
        let i = Instance {
            root: "/tmp/notes".to_string(),
            port: 8090,
            token: "t".to_string(),
            pid: 42,
            started_at: "2026-11-09 10:00:00".to_string(),
        };
        let text = serde_json::to_string(&i).unwrap();
        assert_eq!(serde_json::from_str::<Instance>(&text).unwrap(), i);
    }
}
