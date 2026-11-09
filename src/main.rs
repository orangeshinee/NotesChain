//! NotesChain —— 极度轻量、本地优先的 Markdown 笔记工具。
//!
//! 竖切版本（v0.1）：扫描 → 待办提取 → 首页勾选 → 三级定位回写。
//! CLI 入口（设计文档）：同一二进制，带子命令时走 CLI（只调 HTTP 接口），
//! 不带子命令时保持原服务启动路径不变。

mod api;
mod cli;
mod git;
mod index;
mod markdown;
mod schedule;
mod state;
mod token_file;
mod write;
mod todo_write;
mod watch;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "noteschain", version, about = "极度轻量、本地优先的 Markdown 笔记工具")]
struct Args {
    /// 子命令（CLI 模式）；不带子命令时启动服务
    #[command(subcommand)]
    command: Option<cli::Command>,

    /// 笔记根目录（服务模式必填；CLI 模式可选，用于校验服务端目录）
    // 注意：不能用 `required_unless_present = "command"`——全局参数的必填校验
    // 会在子命令作用域里重复执行，那里看不到父级的 subcommand，导致 CLI 模式
    // 也被强制要求 --dir；所以这里放宽为 Option，缺失时由 run_server 自行报错。
    #[arg(long, value_name = "DIR", global = true)]
    dir: Option<PathBuf>,

    /// 监听端口（只绑 127.0.0.1）
    #[arg(long, default_value_t = 8090, global = true)]
    port: u16,

    /// 以 JSON 输出（CLI 模式）
    #[arg(long, default_value_t = false, global = true)]
    json: bool,

    /// 跳过 token（仅当服务端本身以 --no-token 运行时使用）
    #[arg(long, default_value_t = false, global = true)]
    no_token: bool,

    /// 启用双链 [[目标]]（实验性，PRD 3.5 默认关闭；服务模式）
    #[arg(long, default_value_t = false)]
    links: bool,

    /// 关闭 Git 自动提交（PRD 6.4：同步盘等场景可单独关闭，其余功能保留；服务模式）
    #[arg(long, default_value_t = false)]
    no_git: bool,

    /// Git 自动提交防抖间隔（秒），默认 300（停止变更 5 分钟后合并提交一次；服务模式）
    #[arg(long, default_value_t = 300)]
    commit_debounce: u64,

    /// 连续编辑时强制提交间隔（秒），默认 1800（30 分钟；服务模式）
    #[arg(long, default_value_t = 1800)]
    commit_force: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // clap 默认把用法错误 exit 2；CLI 退出码约定里 2 专指“服务端未启动”，
    // 故这里自行翻译：--help/--version 走 0，其余用法错误走 1。
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => {
            let code = if e.use_stderr() { 1 } else { 0 };
            let _ = e.print();
            std::process::exit(code);
        }
    };

    if let Some(command) = args.command {
        let code = cli::run(args.dir, args.port, args.json, args.no_token, command);
        std::process::exit(code);
    }

    run_server(args).await
}

async fn run_server(args: Args) -> anyhow::Result<()> {
    let Some(dir) = args.dir else {
        eprintln!("错误：服务模式需要 --dir <笔记目录>（例如：noteschain --dir D:\\MyNotes）");
        std::process::exit(cli::EXIT_OTHER);
    };
    let state = state::AppState::new(
        dir,
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

    // 绑定成功后再写 token 文件（CLI 靠它拿当前 token）；写失败只告警，不影响服务。
    let token_written = if state.token.is_empty() {
        false
    } else {
        match token_file::write_token_file(&state.token, args.port) {
            Ok(_) => true,
            Err(e) => {
                eprintln!("[warn] token 文件写入失败（CLI 将无法自动鉴权）：{e}");
                false
            }
        }
    };

    println!("NotesChain 已启动");
    println!("  索引：{file_count} 个 Markdown 文件，{todo_count} 条未完成待办");
    if state.token.is_empty() {
        println!("  打开：http://127.0.0.1:{}/", args.port);
        println!("  ⚠ 已用 --no-token 关闭鉴权，仅建议在个人独占机器使用");
    } else {
        println!("  打开：http://127.0.0.1:{}/?token={}", args.port, state.token);
        println!("  （token 只存在于本次进程，重启后需用新地址重新打开）");
    }

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;

    // 正常退出（Ctrl+C / SIGTERM）时清理 token 文件；崩溃残留靠下次启动覆盖。
    if token_written {
        token_file::remove_token_file();
    }
    result?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("安装 Ctrl+C 处理失败");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("安装 SIGTERM 处理失败")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
