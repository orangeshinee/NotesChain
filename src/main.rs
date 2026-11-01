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
    #[arg(long, default_value_t = 8080)]
    port: u16,

    /// 启用双链 [[目标]]（实验性，PRD 3.5 默认关闭）
    #[arg(long, default_value_t = false)]
    links: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let state = state::AppState::new(args.dir, args.links)?;

    let warnings = state.rebuild_index().await?;
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
    println!("  笔记目录：{}", state.root.display());
    println!("  索引：{file_count} 个 Markdown 文件，{todo_count} 条未完成待办");
    println!("  打开：http://127.0.0.1:{}/?token={}", args.port, state.token);
    println!("  （token 只存在于本次进程，重启后需用新地址重新打开）");

    axum::serve(listener, app).await?;
    Ok(())
}
