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
    /// 规范化后的笔记根目录
    pub root: PathBuf,
    /// 本次进程的启动 token（不落盘，PRD 9.1）
    pub token: String,
    pub index: RwLock<Index>,
    /// Git 自动提交调度
    pub git: Arc<Git>,
    /// 索引修订号：外部改动后自增，前端轮询它决定是否刷新
    pub revision: std::sync::atomic::AtomicU64,
    /// 按文件串行化写入（PRD 5.3 第 7 条）
    file_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl AppState {
    pub fn new(root: PathBuf) -> anyhow::Result<Arc<Self>> {
        let root = root
            .canonicalize()
            .with_context(|| format!("笔记目录不可访问: {}", root.display()))?;
        // Windows 的 canonicalize 会返回 `\\?\F:\...` 扩展路径，git 不认这种路径，
        // 显示出来也难看；这里统一去掉前缀。
        let root = simplify(root);
        if !root.is_dir() {
            anyhow::bail!("笔记目录不是目录: {}", root.display());
        }
        let git = Git::start(root.clone());
        Ok(Arc::new(AppState {
            root,
            token: new_token(),
            index: RwLock::new(Index::default()),
            git,
            revision: std::sync::atomic::AtomicU64::new(0),
            file_locks: Mutex::new(HashMap::new()),
        }))
    }

    /// 重建内存索引，返回扫描告警（非 UTF-8 文件、遍历失败等）。
    pub async fn rebuild_index(&self) -> anyhow::Result<Vec<String>> {
        let index = Index::scan(&self.root)?;
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
        let joined = self.root.join(candidate);
        // 已存在则直接规范化；不存在则规范化其父目录，防软链接跳出
        let resolved = if joined.exists() {
            joined.canonicalize()?
        } else {
            let parent = joined.parent().context("路径没有父目录")?;
            let parent = if parent.exists() {
                parent.canonicalize()?
            } else {
                parent.to_path_buf()
            };
            parent.join(joined.file_name().context("路径缺少文件名")?)
        };
        let resolved = simplify(resolved);
        if !resolved.starts_with(&self.root) {
            anyhow::bail!("路径越界");
        }
        Ok(resolved)
    }

    /// 相对路径（`/` 分隔）
    pub fn relative(&self, abs: &Path) -> String {
        abs.strip_prefix(&self.root)
            .unwrap_or(abs)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// 去掉 Windows 扩展长度路径前缀 `\\?\`。
fn simplify(path: PathBuf) -> PathBuf {
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
        AppState::new(dir.to_path_buf()).unwrap()
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
