# 定向写入 CLI（基于 v0.1.3 的增量）

日期：2026-11-09 ｜ 基线：`origin/master` `655ea368`（v0.1.3）

## 背景

v0.1.3 的 PR #2 已经落地一套完整 CLI：`today` / `todo (add|list|done)` / `search` / `schedule <range>` / `note add` / `git status`，配套 `src/token_file.rs`（token 文件 0600、跨平台 ACL、`NC_TOKEN` 优先级）、`ureq` HTTP 客户端、`--json` 原样透传、退出码 `0/1/2/3/4` 与 graceful shutdown。

此前 PR #6 基于过期的 master 快照另做了一套 CLI，与之重复且产生 add/add 冲突，已关闭并重做。本次**只补缺口**，不重建骨架。

**远端仍未有的能力**：指定文件 + 指定标题写入。`todos/append` 虽有 `path` 但 CLI 不透传；`notes/append` 的 `target` 只追加到文件尾；`schedule/append` 写死今日日记且 CLI 没有追加命令。

## 复用与丢弃

| 项 | 处理 |
| --- | --- |
| `src/cli.rs` 的命令结构、鉴权、`Client::request`、退出码、`--json` | 复用，只加参数 |
| `src/token_file.rs` / `src/main.rs` 的 clap 与 shutdown | 不动 |
| `Cargo.toml` 依赖 | 不动（`ureq` 已在） |
| v1 的实例登记表 / 手写 HTTP 客户端 / 独立退出码 | 丢弃 |
| v1 的 `add todo|schedule|line` 命令名 | 改成远端已有的 `todo add` / `note add` / `schedule --add` |

## 增量契约

三个 append 接口共用 `AppendRequest`：`text` / `path`（`target` 为旧别名）/ `today` / `section` / `create` / `unique` / `dry_run`，统一走 `append_impl`。

- `path` 可只写**文件名**，服务端按索引短名解析：唯一命中自动展开、多个命中 `400` 列出候选不静默选一个、零命中日期形按目录模型展开 / 其余补 `.md` 走旧新建逻辑；含 `/` 的完整路径原样透传，旧调用方零影响。
- `today` 显式指向今日日记，与 `path` 互斥。
- `section` 接受完整标题行或纯标题名（自动补 `## `）；拒绝含换行。
- todo 缺省 `## 待办`；schedule 缺省候选列表 `["## 日程", "## 今日日程"]` 且只接受这两个；note 不传则文件尾追加（旧行为）。
- `create` 不传时沿用 v0.1.3 行为；显式 `false` 返回 `404 not_found`。
- `unique` 命中返回 `action: "skipped"`；`dry_run` 零写入。
- 返回体在旧 `{path, line, version}` 上追加 `ok` / `action` / `section` / `text`，旧调用方不受影响。
- CLI 侧 `todo add` / `note add` 加 `--file` / `--today` / `--section` / `--create` / `--unique` / `--dry-run`；`schedule` 保留 `range` 位置参数并新增 `--add` 等同组参数。退出码与 `--json` 全部沿用 v0.1.3。

## 写入层 `src/write.rs`

替换远端 `insert_into_section`，修掉五处既有问题：CRLF 与混合行尾被整篇统一、BOM 口径、代码块内同名标题误命中标题与段尾、H3 及以上标题的段尾只按 H2 截断、段不存在时 `trim_end()` 抹掉原文尾空格。实现按行保留原始 eol 与 BOM，只新增必要的一行。

`notes/append` 不传 `section` 时走 `append_at_end`，保持旧的文件尾追加语义。

## 验证

- `cargo build --release`、`cargo test`、`cargo clippy --all-targets -- -D warnings` 全绿。
- 端到端脚本以真实进程覆盖：旧命令零破坏、`--json` 透传、定向写入、dry-run 零写入、unique、日程白名单与老笔记别名、CRLF/BOM/混合行尾、H3 段尾、换行注入拒绝、路径越界、尾空格保留、token 文件权限与落点、revision 递增。
- **未跑** `pwsh -File tools/release-gate.ps1`：开发机没有 pwsh，接口冒烟、边界场景与浏览器探针需在 Windows 侧或带 pwsh 的 CI 上补跑。

## 已知行为

server 正常退出会删除 token 文件，此时 CLI 在读 token 阶段即报错（退出码 1），而不是走到「服务端未启动」（退出码 2）。这是 v0.1.3 既有行为，本次未改。
