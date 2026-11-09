//! CLI token 文件：服务端启动时写入，CLI 每次调用时读取（设计文档 §5）。
//!
//! 四条硬要求：
//! 1. 位置在笔记目录之外（Windows `%LOCALAPPDATA%\NotesChain\token`、
//!    Linux/macOS `~/.config/noteschain/token`）；
//! 2. 权限 0600 语义（Unix 显式 0o600；Windows 用 icacls 去掉继承、只授权当前用户；
//!    收紧失败仅告警，不影响服务启动）；
//! 3. 内容带 token + 端口 + 启动时间，CLI 读后先校验端口一致；
//! 4. 服务端正常退出时删除；崩溃残留靠下次启动覆盖。

use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenFile {
    pub token: String,
    pub port: u16,
    /// 服务端启动时间（Unix 秒）
    pub started_at: u64,
}

/// 配置目录（不含文件名）：Windows `%LOCALAPPDATA%\NotesChain`，
/// 其余平台 `$XDG_CONFIG_HOME/noteschain` 或 `~/.config/noteschain`。
fn config_base() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")?;
        Some(PathBuf::from(base).join("NotesChain"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg).join("noteschain"));
            }
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config").join("noteschain"))
    }
}

/// token 文件路径（配置目录下，不在笔记目录内）。
pub fn token_file_path() -> Option<PathBuf> {
    config_base().map(|base| token_file_path_in(&base))
}

/// 在给定基目录下计算 token 文件路径（测试用，避免触碰真实配置目录）。
pub fn token_file_path_in(base: &Path) -> PathBuf {
    base.join("token")
}

/// 服务端启动时写 token 文件（不存在父目录则创建；已存在则覆盖）。
pub fn write_token_file(token: &str, port: u16) -> std::io::Result<PathBuf> {
    let path = token_file_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "无法确定配置目录"))?;
    write_token_file_at(&path, token, port)
}

/// 写入指定路径的 token 文件（真正实现；测试可传临时路径）。
pub fn write_token_file_at(path: &Path, token: &str, port: u16) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let body = format!(
        "# NotesChain CLI token（本机当前 token，勿外传）\n\
         token={token}\n\
         port={port}\n\
         started_at={started_at}\n"
    );
    // 0600：仅属主可读写；文件已存在时也要收紧权限。
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::fs::PermissionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(body.as_bytes())?;
        // 已存在文件的权限不受 OpenOptionsExt::mode 影响，显式再设一次。
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let mut file = std::fs::File::create(path)?;
        file.write_all(body.as_bytes())?;
    }
    // Windows 默认 ACL 可能让同机其他账户读到 token；去掉继承、只授权当前用户（0600 语义）。
    // icacls 缺失或失败只告警：不能因为收紧权限失败就让服务起不来。
    #[cfg(windows)]
    {
        if let Err(e) = restrict_to_current_user(path) {
            eprintln!("[warn] token 文件权限收紧失败（多用户机器上其他账户可能读取）：{e}");
        }
    }
    Ok(path.to_path_buf())
}

/// Windows：用 icacls 移除继承并只给当前用户完全控制。
/// 注意不能用 `*`（icacls 会把它当成「后面跟 SID」而报 invalid），要用实际账户名。
#[cfg(windows)]
fn restrict_to_current_user(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    let user = std::env::var("USERNAME").map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "缺少 USERNAME 环境变量")
    })?;
    let account = match std::env::var("USERDOMAIN") {
        Ok(dom) if !dom.is_empty() => format!("{dom}\\{user}"),
        _ => user,
    };
    let grant = format!("{account}:(F)");
    let out = Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(&grant)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// 解析 token 文件内容（跳过 `#` 注释与空行；缺字段即报错）。
pub fn parse_token_file(text: &str) -> Result<TokenFile, String> {
    let mut token: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut started_at: Option<u64> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("token 文件格式错误：{line}"))?;
        match key.trim() {
            "token" => token = Some(value.trim().to_string()),
            "port" => {
                port = Some(
                    value
                        .trim()
                        .parse()
                        .map_err(|_| format!("token 文件端口非法：{value}"))?,
                )
            }
            "started_at" => {
                started_at = Some(
                    value
                        .trim()
                        .parse()
                        .map_err(|_| format!("token 文件启动时间非法：{value}"))?,
                )
            }
            _ => { /* 忽略未知字段，向后兼容 */ }
        }
    }
    Ok(TokenFile {
        token: token.ok_or_else(|| "token 文件缺少 token 字段".to_string())?,
        port: port.ok_or_else(|| "token 文件缺少 port 字段".to_string())?,
        started_at: started_at.unwrap_or(0),
    })
}

/// CLI 侧读 token 文件。
pub fn read_token_file() -> Result<TokenFile, String> {
    let path = token_file_path().ok_or_else(|| "无法确定配置目录".to_string())?;
    read_token_file_at(&path)
}

/// 读取指定路径的 token 文件（测试用）。
pub fn read_token_file_at(path: &Path) -> Result<TokenFile, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取 token 文件 {}：{e}", path.display()))?;
    parse_token_file(&text)
}

/// 服务端退出时清理 token 文件（失败静默：残留文件下次启动会被覆盖）。
pub fn remove_token_file() {
    if let Some(path) = token_file_path() {
        remove_token_file_at(&path);
    }
}

/// 删除指定路径的 token 文件（失败静默）。
pub fn remove_token_file_at(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        // 放在 crate 的 target/ 下而不是系统 %TEMP%：某些受限/沙箱环境里子进程对系统
        // 临时目录没有写权限（会在 create_dir_all 处 PermissionDenied），而 target/ 一定
        // 可写，且随 `cargo clean` 一起清理，不污染系统临时目录。
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-tmp")
            .join(format!(
                "nc-token-test-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrip_and_permissions() {
        // 用显式临时目录，避免像旧实现那样依赖 XDG_CONFIG_HOME（Windows 会忽略它，
        // 导致写到真实 %LOCALAPPDATA%，测试失败并覆盖真实 token 文件）。
        let dir = temp_dir("roundtrip");
        let path = token_file_path_in(&dir);
        let written = write_token_file_at(&path, "aabbcc", 8099).unwrap();
        assert_eq!(written, path);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "token 文件必须是 0600");
        }
        let parsed = read_token_file_at(&path).unwrap();
        assert_eq!(parsed.token, "aabbcc");
        assert_eq!(parsed.port, 8099);
        assert!(parsed.started_at > 0);
        remove_token_file_at(&path);
        assert!(!path.exists(), "退出清理应删除 token 文件");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_rejects_malformed() {
        assert!(parse_token_file("token=abc\n").is_err(), "缺 port 应报错");
        assert!(parse_token_file("token=abc\nport=xx\n").is_err(), "非法端口应报错");
        assert!(parse_token_file("garbage line\n").is_err(), "无 = 的行应报错");
        let ok = parse_token_file("# 注释\ntoken=abc\nport=8090\nstarted_at=123\n").unwrap();
        assert_eq!(ok.token, "abc");
        assert_eq!(ok.port, 8090);
        assert_eq!(ok.started_at, 123);
    }
}
