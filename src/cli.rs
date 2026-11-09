//! CLI 子命令：定向写入、实例登记表、最小 HTTP 客户端。
//!
//! 设计见 docs/plan-2026-11-09-cli-write.md。要点：
//! - 登记表放用户级目录，token 绝不写进笔记目录（会被 Git 自动提交收走）。
//! - CLI 不自动拉起 server，也不自动重试，避免重复落盘。
//! - 只连 127.0.0.1，无 TLS，不引第三方 HTTP 依赖，守住单文件体积。

use std::collections::HashSet;
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

/// (退出码, 稳定错误码, 可直接展示的中文消息)
type Failure = (i32, String, String);

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
/// 回退必须用绝对路径：相对 "." 会落在进程 cwd，若 cwd 就是 --dir，
/// token 就会被 Git 自动提交收进版本历史。
fn registry_path() -> PathBuf {
    #[cfg(windows)]
    let path = {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join("NotesChain").join("instances.json")
    };
    #[cfg(not(windows))]
    let path = {
        if let Some(xdg) = std::env::var_os("XDG_RUNTIME_DIR") {
            PathBuf::from(xdg).join("noteschain").join("instances.json")
        } else {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            home.join(".cache").join("noteschain").join("instances.json")
        }
    };
    path
}

fn load_registry() -> Vec<Instance> {
    let text = match std::fs::read_to_string(registry_path()) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            eprintln!("[cli] 读取实例登记表失败: {err}");
            return Vec::new();
        }
    };
    match serde_json::from_str(&text) {
        Ok(list) => list,
        Err(err) => {
            eprintln!("[cli] 实例登记表损坏（{err}），本次按空表处理");
            Vec::new()
        }
    }
}

/// 原子写：同目录临时文件 + rename，避免 CLI 读到写了一半的登记表。
fn save_registry(list: &[Instance]) -> Result<(), String> {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = path.with_file_name(format!(".instances.{}.tmp", std::process::id()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        // 覆盖已存在的宽松权限文件（mode() 只对新建生效）
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        // Windows 上 rename 覆盖已存在文件会失败，退回直接写；
        // 权限由 %LOCALAPPDATA% 的 ACL 决定，见 README 的限制说明。
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn register(instance: &Instance) {
    let mut list = load_registry();
    // 同一端口只可能有一个 server，按端口覆盖可清掉崩溃残留与同端口换目录的旧条目
    list.retain(|i| i.port != instance.port);
    list.push(instance.clone());
    if let Err(err) = save_registry(&list) {
        eprintln!("[cli] 登记实例失败: {err}");
    }
}

pub fn unregister(instance: &Instance) {
    let mut list = load_registry();
    list.retain(|i| !(i.port == instance.port && i.pid == instance.pid));
    if let Err(err) = save_registry(&list) {
        eprintln!("[cli] 注销实例失败: {err}");
    }
}

/// 端口通 +（Unix）pid 对应的进程还在，才算活实例。
fn alive(instance: &Instance) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], instance.port));
    if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err() {
        return false;
    }
    #[cfg(unix)]
    if instance.pid > 0 && !PathBuf::from(format!("/proc/{}", instance.pid)).exists() {
        return false;
    }
    true
}

/// Windows 的 canonicalize 带 `\\?\` 前缀，server 侧已用 simplify 去掉，这里对齐。
fn normalize_dir(raw: &str) -> String {
    let canon = std::fs::canonicalize(raw)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| raw.to_string());
    const PREFIX: &str = r"\\?\";
    canon.strip_prefix(PREFIX).unwrap_or(&canon).to_string()
}

fn select(args: &AddArgs) -> Result<Instance, Failure> {
    let mut list: Vec<Instance> = load_registry().into_iter().filter(alive).collect();
    if let Some(dir) = &args.dir {
        let canon = normalize_dir(dir);
        list.retain(|i| normalize_dir(&i.root) == canon || i.root == *dir);
    }
    if let Some(port) = args.port {
        list.retain(|i| i.port == port);
    }
    if list.is_empty() {
        return Err((
            EXIT_LOCATE,
            "locate_failed".into(),
            "找不到运行中的 NotesChain 实例。请先启动服务；多实例时用 --dir / --port 消歧。"
                .into(),
        ));
    }
    let roots: HashSet<&str> = list.iter().map(|i| i.root.as_str()).collect();
    if roots.len() < list.len() {
        // 两个 server 各持独立内存锁与索引，并发写同一文件会丢更新
        return Err((
            EXIT_LOCATE,
            "locate_failed".into(),
            "有多个实例共用同一笔记根：并发写会丢更新，请只保留一个 server，或用 --dir 指定。"
                .into(),
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
            "locate_failed".into(),
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
        pos = end.checked_add(2).ok_or_else(|| "chunked 偏移溢出".to_string())?;
        if size == 0 {
            break;
        }
        // 先做有上限的 checked 运算，避免 release 下整数回绕绕过边界检查
        let stop = pos
            .checked_add(size)
            .ok_or_else(|| "chunked 尺寸溢出".to_string())?;
        let next = stop
            .checked_add(2)
            .ok_or_else(|| "chunked 尺寸溢出".to_string())?;
        if stop > data.len() || next > data.len() {
            return Err("chunked 数据不完整".to_string());
        }
        out.extend_from_slice(&data[pos..stop]);
        pos = next;
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

fn execute(kind: Kind, args: &AddArgs) -> Result<Value, Failure> {
    let text = args.text.trim();
    if text.is_empty() {
        return Err((EXIT_USAGE, "bad_request".into(), "内容为空（--text 必填）".into()));
    }
    if text.contains('\n') || text.contains('\r') {
        return Err((
            EXIT_USAGE,
            "bad_request".into(),
            "一次只能添加一条（--text 不能含换行）".into(),
        ));
    }
    if kind == Kind::Line && args.section.is_none() {
        return Err((EXIT_USAGE, "bad_request".into(), "add line 需要 --section".into()));
    }
    if let Some(section) = &args.section {
        if section.contains('\n') || section.contains('\r') {
            return Err((
                EXIT_USAGE,
                "bad_request".into(),
                "标题不能含换行".into(),
            ));
        }
    }

    let instance = select(args)?;
    let token = args.token.clone().unwrap_or_else(|| instance.token.clone());
    if token.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err((EXIT_USAGE, "bad_request".into(), "token 含非法控制字符".into()));
    }
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
        .map_err(|e| (EXIT_LOCATE, "locate_failed".into(), format!("请求失败：{e}")))?;
    if !(200..300).contains(&status) {
        let code = value["error"]["code"].as_str().unwrap_or("unknown").to_string();
        let message = value["error"]["message"]
            .as_str()
            .unwrap_or("请求失败")
            .to_string();
        let exit = match code.as_str() {
            "bad_request" | "bad_path" => EXIT_USAGE,
            "not_found" => EXIT_NOT_FOUND,
            "unauthorized" => EXIT_LOCATE,
            _ => EXIT_CONFLICT,
        };
        return Err((exit, code, message));
    }
    // 2xx 但缺少必要字段时不能当成写入成功，否则脚本按退出码会误判
    if value.get("ok") != Some(&Value::Bool(true))
        || value.get("action").is_none()
        || value.get("path").is_none()
    {
        return Err((
            EXIT_CONFLICT,
            "internal".into(),
            "服务端响应缺少必要字段，无法确认写入结果".into(),
        ));
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
        Err((exit, code, message)) => {
            if args.json {
                // 错误 JSON 走 stderr，stdout 只留给成功结果（设计文档 §7.3）
                eprintln!("{}", json!({ "ok": false, "code": code, "message": message }));
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
    fn registry_path_is_absolute_user_level() {
        let path = registry_path();
        assert!(path.is_absolute(), "{}", path.display());
        assert_eq!(path.file_name().unwrap(), "instances.json");
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
    fn chunked_overflow_is_rejected_not_panicking() {
        let resp = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\nx\r\n0\r\n\r\n";
        assert!(parse_response(resp).is_err());
    }

    #[test]
    fn normalize_dir_strips_windows_prefix() {
        let raw = r"\\?\F:\MyNotes";
        assert_eq!(normalize_dir(raw), r"F:\MyNotes");
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
