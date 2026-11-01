//! 应用状态：根目录、启动 token、内存索引、按文件写锁。

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use rand::RngCore;
use tokio::sync::{Mutex, RwLock};

use crate::git::Git;
use crate::index::Index;

pub struct AppState {
    /// 规范化后的项目根目录（`--dir` 指向它，git 仓库一般在这里）
    pub root: PathBuf,
    /// Markdown 实际所在目录：`<root>/notes`（存在时），否则就是 root
    pub notes_root: PathBuf,
    /// 附件目录：`<root>/assets`（存在时），否则 notes_root/assets
    pub assets_root: PathBuf,
    /// 本次进程的启动 token（不落盘，PRD 9.1）
    pub token: String,
    pub index: RwLock<Index>,
    /// Git 自动提交调度
    pub git: Arc<Git>,
    /// 索引修订号：外部改动后自增，前端轮询它决定是否刷新
    pub revision: std::sync::atomic::AtomicU64,
    /// 双链开关（PRD 3.5：默认关闭）
    pub links: bool,
    /// 按文件串行化写入（PRD 5.3 第 7 条）
    file_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl AppState {
    pub fn new(root: PathBuf, links: bool, git_enabled: bool) -> anyhow::Result<Arc<Self>> {
        let root = root
            .canonicalize()
            .with_context(|| format!("笔记目录不可访问: {}", root.display()))?;
        // Windows 的 canonicalize 会返回 `\\?\F:\...` 扩展路径，git 不认这种路径，
        // 显示出来也难看；这里统一去掉前缀。
        let root = simplify(root);
        if !root.is_dir() {
            anyhow::bail!("笔记目录不是目录: {}", root.display());
        }
        // 目录模型（v0.6）：MyNotes/{notes, assets}；兼容旧布局（md 直接在 root 下）
        let notes_root = if root.join("notes").is_dir() {
            root.join("notes")
        } else {
            root.clone()
        };
        let assets_root = if root.join("assets").is_dir() {
            root.join("assets")
        } else {
            notes_root.join("assets")
        };
        let git = Git::start(root.clone(), git_enabled);
        Ok(Arc::new(AppState {
            root,
            notes_root,
            assets_root,
            token: new_token(),
            index: RwLock::new(Index::default()),
            git,
            revision: std::sync::atomic::AtomicU64::new(0),
            links,
            file_locks: Mutex::new(HashMap::new()),
        }))
    }

    /// 重建内存索引，返回扫描告警（非 UTF-8 文件、遍历失败等）。
    pub async fn rebuild_index(&self) -> anyhow::Result<Vec<String>> {
        let index = Index::scan(&self.notes_root)?;
        let warnings = index.warnings.clone();
        *self.index.write().await = index;
        Ok(warnings)
    }

    pub async fn lock_for(&self, rel: &str) -> Arc<Mutex<()>> {
        let mut locks = self.file_locks.lock().await;
        locks
            .entry(rel.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// 把接口传来的相对路径解析为根目录内的绝对路径（PRD 9.2）。
    pub fn resolve(&self, rel: &str) -> anyhow::Result<PathBuf> {
        if rel.is_empty() {
            anyhow::bail!("路径为空");
        }
        let normalized = rel.replace('\\', "/");
        let candidate = Path::new(&normalized);
        if candidate.is_absolute() {
            anyhow::bail!("不接受绝对路径");
        }
        for comp in candidate.components() {
            match comp {
                Component::Normal(_) | Component::CurDir => {}
                _ => anyhow::bail!("路径越界"),
            }
        }
        let joined = self.notes_root.join(candidate);
        // 规范化：向上找到最近的已存在祖先做 canonicalize，再把剩余层级接回去，
        // 这样即使中间层级还不存在，也不会绕过软链接校验。
        let mut existing = joined.clone();
        let mut suffix: Vec<std::ffi::OsString> = Vec::new();
        while !existing.exists() {
            match existing.file_name().map(|n| n.to_os_string()) {
                Some(name) => {
                    suffix.push(name);
                    match existing.parent() {
                        Some(parent) => existing = parent.to_path_buf(),
                        None => break,
                    }
                }
                None => break,
            }
        }
        let mut resolved = existing.canonicalize()?;
        for part in suffix.iter().rev() {
            resolved.push(part);
        }
        let resolved = simplify(resolved);
        if !resolved.starts_with(&self.notes_root) {
            anyhow::bail!("路径越界");
        }
        Ok(resolved)
    }

    /// 相对路径（相对 notes 根，`/` 分隔）
    pub fn relative(&self, abs: &Path) -> String {
        abs.strip_prefix(&self.notes_root)
            .unwrap_or(abs)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// 去掉 Windows 扩展长度路径前缀 `\\?\`。
pub(crate) fn simplify(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_for(dir: &Path) -> Arc<AppState> {
        AppState::new(dir.to_path_buf(), false, true).unwrap()
    }

    #[test]
    fn rejects_traversal() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp");
        let dir = base.join("state-test");
        std::fs::create_dir_all(&dir).unwrap();
        let st = state_for(&dir);
        assert!(st.resolve("../secret.txt").is_err());
        assert!(st.resolve("a/../../b.md").is_err());
        assert!(st.resolve("/etc/passwd").is_err());
        assert!(st.resolve("ok/note.md").is_ok());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn token_is_random_hex() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }
}
