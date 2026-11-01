//! Git 自动提交（PRD 第 6 章）：启动检测、自动 init、30 秒防抖合并提交。
//!
//! 只用命令行 git，不引 libgit2 —— 守住单文件与内存目标；git 是用时起的短命进程。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::{mpsc, Mutex};

const DEBOUNCE: Duration = Duration::from_secs(30);
const FORCE_AFTER: Duration = Duration::from_secs(300);

#[derive(Debug, Default, Clone)]
pub struct Runtime {
    pub last_commit: Option<String>,
    pub last_error: Option<String>,
    pub failures: u32,
    pub disabled: bool,
    pub pending_files: usize,
}

pub struct Git {
    pub root: PathBuf,
    pub available: bool,
    pub repo_root: Option<PathBuf>,
    tx: Option<mpsc::Sender<()>>,
    runtime: Arc<Mutex<Runtime>>,
}

#[derive(Debug, Serialize)]
pub struct GitStatus {
    pub available: bool,
    pub is_repo: bool,
    pub repo_root: Option<String>,
    pub last_commit: Option<String>,
    pub pending_files: usize,
    pub last_error: Option<String>,
    pub disabled: bool,
    pub note: &'static str,
}

impl Git {
    /// 启动检测 + 自动 init + 起提交协程。
    pub fn start(root: PathBuf) -> Arc<Git> {
        let available = git_available();
        let mut repo_root = if available { find_repo_root(&root) } else { None };

        if available && repo_root.is_none() {
            match init_repo(&root) {
                Ok(()) => repo_root = Some(root.clone()),
                Err(err) => eprintln!("[git] 自动 init 失败: {err}"),
            }
        }

        if available {
            if let Some(repo) = &repo_root {
                ensure_gitignore(&root, repo);
            }
        }

        let (tx, rx) = mpsc::channel(64);
        let runtime = Arc::new(Mutex::new(Runtime::default()));

        if let Some(repo) = repo_root.clone() {
            // 单元测试里没有 Tokio 运行时：不启动提交协程即可
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let worker_root = root.clone();
                let state = runtime.clone();
                handle.spawn(async move {
                    worker(rx, worker_root, repo, state).await;
                });
            }
        }

        Arc::new(Git {
            root,
            available,
            repo_root,
            tx: Some(tx),
            runtime,
        })
    }

    /// 有写入发生：进入（或延长）防抖窗口。
    pub fn notify(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(());
        }
    }

    pub async fn status(&self) -> GitStatus {
        let rt = self.runtime.lock().await;
        let pending = if self.available {
            self.repo_root
                .as_ref()
                .map(|repo| count_pending(&self.root, repo))
                .unwrap_or(0)
        } else {
            0
        };
        GitStatus {
            available: self.available,
            is_repo: self.repo_root.is_some(),
            repo_root: self.repo_root.as_ref().map(|p| p.display().to_string()),
            last_commit: rt.last_commit.clone(),
            pending_files: pending,
            last_error: rt.last_error.clone(),
            disabled: rt.disabled,
            note: "仅本地版本历史，无远程备份",
        }
    }
}

async fn worker(
    mut rx: mpsc::Receiver<()>,
    scope_root: PathBuf,
    repo_root: PathBuf,
    state: Arc<Mutex<Runtime>>,
) {
    loop {
        // 等第一次变更
        if rx.recv().await.is_none() {
            return;
        }
        let first = Instant::now();
        let mut last = Instant::now();

        // 防抖：最后一次变更后 30 秒，或累计 5 分钟强制提交
        loop {
            let until_debounce = DEBOUNCE.saturating_sub(last.elapsed());
            let until_force = FORCE_AFTER.saturating_sub(first.elapsed());
            let wait = until_debounce.min(until_force);
            if wait.is_zero() {
                break;
            }
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(()) => last = Instant::now(),
                    None => return,
                },
                _ = tokio::time::sleep(wait) => break,
            }
        }

        let scope = scope_root.clone();
        let repo = repo_root.clone();
        let result = tokio::task::spawn_blocking(move || run_commit(&scope, &repo)).await;

        let mut rt = state.lock().await;
        match result {
            Ok(Ok(outcome)) => {
                rt.failures = 0;
                rt.last_error = None;
                if let Some((count, at)) = outcome {
                    rt.last_commit = Some(at);
                    rt.pending_files = 0;
                    let _ = count;
                }
            }
            Ok(Err(err)) => {
                rt.failures += 1;
                rt.last_error = Some(err);
                if rt.failures >= 3 {
                    rt.disabled = true;
                }
            }
            Err(err) => {
                rt.failures += 1;
                rt.last_error = Some(format!("提交任务异常: {err}"));
                if rt.failures >= 3 {
                    rt.disabled = true;
                }
            }
        }

        if rt.disabled {
            return;
        }
    }
}

/// 返回 `Some((文件数, 提交时间))`；无变更返回 `None`。
fn run_commit(scope_root: &Path, repo_root: &Path) -> Result<Option<(usize, String)>, String> {
    let rel = scope_root
        .strip_prefix(repo_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let rel = if rel.is_empty() { ".".to_string() } else { rel };

    let add = Command::new("git")
        .args(["add", "-A", "--", &rel])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("git add 失败: {e}"))?;
    if !add.status.success() {
        return Err(format!(
            "git add 失败: {}",
            String::from_utf8_lossy(&add.stderr).trim()
        ));
    }

    let status = Command::new("git")
        .args(["status", "--porcelain", "--", &rel])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("git status 失败: {e}"))?;
    let changed = String::from_utf8_lossy(&status.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    if changed == 0 {
        return Ok(None);
    }

    let now = chrono::Local::now();
    let message = format!("auto: {} ({} files)", now.format("%Y-%m-%d %H:%M"), changed);

    let mut cmd = Command::new("git");
    if !has_identity(repo_root) {
        cmd.args([
            "-c",
            "user.name=NotesChain",
            "-c",
            "user.email=notes@local",
        ]);
    }
    let commit = cmd
        .args(["commit", "-m", &message])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("git commit 失败: {e}"))?;
    if !commit.status.success() {
        return Err(format!(
            "git commit 失败: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        ));
    }

    Ok(Some((changed, now.format("%Y-%m-%d %H:%M").to_string())))
}

fn count_pending(scope_root: &Path, repo_root: &Path) -> usize {
    let rel = scope_root
        .strip_prefix(repo_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let rel = if rel.is_empty() { ".".to_string() } else { rel };
    Command::new("git")
        .args(["status", "--porcelain", "--", &rel])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count()
        })
        .unwrap_or(0)
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn find_repo_root(root: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn init_repo(root: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("无法执行 git: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn has_identity(repo_root: &Path) -> bool {
    Command::new("git")
        .args(["config", "user.email"])
        .current_dir(repo_root)
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// 首次在笔记目录生成默认 `.gitignore`（PRD 6.4：只忽略临时文件与系统文件，`assets/` 不忽略）。
fn ensure_gitignore(root: &Path, repo_root: &Path) {
    let path = root.join(".gitignore");
    if path.exists() {
        return;
    }
    let default = "\
# NotesChain 自动生成（只忽略临时文件与系统文件；assets/ 不忽略，附件同样进版本历史）
*.tmp
.noteschain.*.tmp
.DS_Store
Thumbs.db
desktop.ini
";
    if std::fs::write(&path, default).is_ok() {
        eprintln!(
            "[git] 已在 {} 生成默认 .gitignore（仓库根：{}）",
            root.display(),
            repo_root.display()
        );
    }
}
