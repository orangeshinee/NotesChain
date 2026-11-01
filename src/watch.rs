//! 文件监听：外部编辑器改了文件，索引与界面跟着更新（PRD 4.6 / 10.3）。
//!
//! 网络盘上 notify 可能失效，前端另有轮询与手动刷新兜底。

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::state::AppState;

const DEBOUNCE: Duration = Duration::from_millis(300);

pub fn start(state: Arc<AppState>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        eprintln!("[watch] 没有运行时，跳过文件监听");
        return;
    };
    let root = state.notes_root.clone();

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(watcher) => watcher,
            Err(err) => {
                eprintln!("[watch] 初始化失败: {err}");
                return;
            }
        };
        if let Err(err) = watcher.watch(&root, RecursiveMode::Recursive) {
            eprintln!("[watch] 监听失败: {err}");
            return;
        }

        loop {
            let first = match rx.recv() {
                Ok(Ok(event)) => event,
                Ok(Err(err)) => {
                    eprintln!("[watch] 事件错误: {err}");
                    continue;
                }
                Err(_) => return,
            };

            let mut paths: Vec<PathBuf> = first.paths.clone();
            // 防抖：300ms 内的事件合并处理
            let deadline = std::time::Instant::now() + DEBOUNCE;
            while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
                match rx.recv_timeout(remaining) {
                    Ok(Ok(event)) => paths.extend(event.paths),
                    Ok(Err(_)) => {}
                    Err(_) => break,
                }
            }

            let mut changed = false;
            for path in paths {
                if !path.starts_with(&root) || !path.is_file() {
                    continue;
                }
                let rel = path
                    .strip_prefix(&root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if !rel.ends_with(".md") || rel.split('/').any(|seg| seg == ".git") {
                    continue;
                }
                let applied = handle.block_on(async {
                    let mut index = state.index.write().await;
                    let before = index
                        .files
                        .iter()
                        .find(|f| f.path == rel)
                        .map(|f| f.version.clone());
                    match index.upsert(&root, &rel) {
                        Ok(()) => {
                            let after = index
                                .files
                                .iter()
                                .find(|f| f.path == rel)
                                .map(|f| f.version.clone());
                            before != after
                        }
                        Err(err) => {
                            eprintln!("[watch] 更新 {rel} 失败: {err}");
                            false
                        }
                    }
                });
                changed |= applied;
            }

            if changed {
                state.revision.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
}
