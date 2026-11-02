//! NotesChain —— 极度轻量、本地优先的 Markdown 笔记工具。
//!
//! 竖切版本（v0.1）：扫描 → 待办提取 → 首页勾选 → 三级定位回写。

mod api;
mod git;
mod index;
mod markdown;
mod schedule;
mod state;
mod watch;
mod todo_write;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "noteschain", version, about = "极度轻量、本地优先的 Markdown 笔记工具")]
struct Args {
    /// 笔记根目录
    #[arg(long, value_name = "DIR")]
    dir: PathBuf,

    /// 监听端口（只绑 127.0.0.1）
    #[arg(long, default_value_t = 8090)]
    port: u16,

    /// 启用双链 [[目标]]（实验性，PRD 3.5 默认关闭）
    #[arg(long, default_value_t = false)]
    links: bool,

    /// 关闭 Git 自动提交（PRD 6.4：同步盘等场景可单独关闭，其余功能保留）
    #[arg(long, default_value_t = false)]
    no_git: bool,

    /// 禁用 token 鉴权（仅建议个人独占机器使用；默认仍校验 token）
    #[arg(long, default_value_t = false)]
    no_token: bool,

    /// Git 自动提交防抖间隔（秒），默认 300（停止变更 5 分钟后合并提交一次）
    #[arg(long, default_value_t = 300)]
    commit_debounce: u64,

    /// 连续编辑时强制提交间隔（秒），默认 1800（30 分钟）
    #[arg(long, default_value_t = 1800)]
    commit_force: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let state = state::AppState::new(
        args.dir,
        args.links,
        !args.no_git,
        args.no_token,
        args.commit_debounce,
        args.commit_force,
    )?;

    let warnings = state.rebuild_index().await?;
    println!("  笔记目录：{}", state.notes_root.display());
    watch::start(state.clone());
    for warning in &warnings {
        eprintln!("[warn] {warning}");
    }

    let file_count = state.index.read().await.files.len();
    let todo_count = state.index.read().await.open_todos().len();

    let app = api::router(state.clone());
    let addr = SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("NotesChain 已启动");
    println!("  索引：{file_count} 个 Markdown 文件，{todo_count} 条未完成待办");
    if state.token.is_empty() {
        println!("  打开：http://127.0.0.1:{}/", args.port);
        println!("  ⚠ 已用 --no-token 关闭鉴权，仅建议在个人独占机器使用");
    } else {
        println!("  打开：http://127.0.0.1:{}/?token={}", args.port, state.token);
        println!("  （token 只存在于本次进程，重启后需用新地址重新打开）");
    }

    axum::serve(listener, app).await?;
    Ok(())
}
