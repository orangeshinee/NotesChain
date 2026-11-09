# NotesChain

极度轻量、本地优先的 Markdown 笔记工具。每天一个文件，保存即落盘，变更进 Git 提交队列——`Chain` 指的就是这条「文件 → 保存 → Git 提交 → 历史版本」的链。

单文件可执行程序（约 3.6 MB），前端资源全部编译期内嵌；笔记是纯 Markdown，没有私有格式。

## 快速开始

```powershell
cargo build --release
.\target\release\noteschain.exe --dir "D:\MyNotes" --port 8090
```

启动后终端会打印一行带 token 的地址（形如 `http://127.0.0.1:8090/?token=...`），浏览器打开即可。首次访问会种下 HttpOnly Cookie 并跳到干净地址；token 只在本次进程内有效，重启需用新地址。

| 参数 | 说明 |
| --- | --- |
| `--dir` | 项目根目录（必填） |
| `--port` | 端口，默认 8090，只绑 `127.0.0.1` |
| `--links` | 启用双链 `[[目标]]`（默认关闭） |
| `--no-git` | 关闭 Git 自动提交，其余功能照常 |
| `--no-token` | 禁用 token 鉴权（仅建议个人独占机器；默认仍校验） |
| `--commit-debounce` | Git 自动提交防抖秒数，默认 **300**（停改 5 分钟合并提交） |
| `--commit-force` | 连续编辑强制提交秒数，默认 **1800** |

目录结构（`--dir` 指向项目根）：

```
MyNotes/
├── notes/        所有 .md（日记在 notes/YYYY/MM/）
├── assets/       图片附件
├── templates/    可选：自定义模板
└── .trash/       删除的笔记（可恢复）
```

> 旧布局兼容：若没有 `notes/`，会把根目录直接当笔记目录。

## 主要能力

- 首页：日程（本日 / 本周 / 本月）+ 全部待办，支持快速添加待办与日程（日程用时间选择器）
- 待办：勾选即按行回写原文件（抗外部插行）、`!p0`–`!p9` 优先级、排序与筛选，控件靠右
- 编辑：CodeMirror Live Preview，默认预览、点「编辑」进入修改；**编辑态正文向预览看齐、frontmatter 折叠成卡片（双击编辑）**；自动保存与冲突处理；标签 `#` 联想
- 外观：设置里可配置字体 / 字号 / 主题
- 搜索：子串匹配 + 标签/状态过滤 + 上下文；标签树多级折叠（含 frontmatter `tags`）
- Git：自动 init、防抖提交（默认 5 分钟，可配）、历史/diff（按文件折叠着色）、按文件回滚
- 安全：启动 token + Cookie、路径穿越防护；`--no-token` 可本地免鉴权

## 定向写入 CLI

服务运行时，另开一个终端即可往指定文件的指定标题下追加一行：

```bash
noteschain add todo     --text "买牛奶 #生活"
noteschain add schedule --text "15:00-16:00 周会 @会议室" --file 2026/11/2026-11-09.md
noteschain add line     --file raw/周报.md --section "## 记录" --text "和财务确认口径"
noteschain add todo     --text "发周报" --dry-run --json
```

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `--text` | 必填 | 单行内容；todo 自动补 `- [ ] `，schedule 自动补 `- `，line 原样 |
| `--file` | 今天的日记 | 相对 notes 根；**不存在时默认拒绝**，`--create` 才新建 |
| `--section` | todo = `## 待办`；schedule = `## 日程` / `## 今日日程`；line 必填 | 接受完整标题行或纯标题名（自动补 `## `）。日程限这两个标题，写进别的段不会出现在日程视图 |
| `--dir` / `--port` | 从实例登记表推断 | 多实例时消歧 |
| `--create` | 关 | 允许新建；日记路径用日记模板，其余用普通笔记模板 |
| `--unique` | 关 | 段内已有完全相同行时跳过 |
| `--dry-run` | 关 | 只预览，零写入 |
| `--json` | 关 | 输出 `{ok, action, path, section, line, version, text}`，供脚本 / cron 解析 |

`action` 为 `inserted` | `skipped` | `dry_run`。退出码：`0` 成功、`2` 参数错误（含路径越界）、`3` 文件不存在（未带 `--create`）、`4` 定位或鉴权失败、`5` 写入冲突或服务端错误。`--json` 模式下成功结果走 stdout，错误 JSON 走 stderr。

服务启动时会在**用户级目录**登记 `{root, port, token, pid}`：Linux/macOS 为 `$XDG_RUNTIME_DIR/noteschain/instances.json`（回退 `~/.cache/noteschain/`，HOME 缺失时用系统临时目录），Windows 为 `%LOCALAPPDATA%\NotesChain\instances.json`。Unix 下文件权限 600；Windows 权限由 `%LOCALAPPDATA%` 的 ACL 决定，这是已知限制。**token 不写入笔记目录**，否则会被 Git 自动提交收进版本历史；它仍只属于本次进程，重启即失效。CLI 从登记表取 port 与 token，不自动拉起 server、不自动重试。多个实例时用 `--dir` / `--port` 指定；**同一笔记根不要起多个 server**，两个进程各持独立锁与索引，并发写会丢更新（CLI 会拒绝这种情况）。

## 验证

```powershell
pwsh -File tools/release-gate.ps1
```

一条命令跑完构建、单元测试、clippy、接口冒烟、边界场景、平替预演与浏览器交互，全绿再提交。

## 发布

推送 `v*` tag 会自动构建 **Windows / Linux 两个平台**的产物并发布到 GitHub Releases。

**打 tag 前必须先同步 bump `Cargo.toml` 与 `Cargo.lock` 里的 `version`**——两处必须一致，且 `v<版本>` 必须等于 tag。CI 会校验，不一致直接失败；只改 `Cargo.toml` 不改 `Cargo.lock` 也会让 `--locked` 构建失败。

```bash
# 1) 同步把 Cargo.toml / Cargo.lock 的 version 改到 0.1.2，提交并推送
# 2) 打 tag
git tag v0.1.2
git push origin v0.1.2
```

| 平台 | 产物 |
| --- | --- |
| Windows x64 | `noteschain-v0.1.2-windows-x64.zip`（exe + README）、同名 `.sha256`、未打包的 `noteschain.exe` |
| Linux x64 | `noteschain-v0.1.2-linux-x64.tar.gz`（二进制 + README）、同名 `.sha256`、未打包的 `noteschain` |

CI（push / PR）在 Windows 与 Ubuntu 双平台跑 release 构建 + 单元测试 + clippy 零警告。已发布的 tag 可以在 Actions → Release → `workflow_dispatch` 填 tag 重跑，补传或覆盖资产。

本地构建 Linux 版：

```bash
cargo build --release --locked
./target/release/noteschain --dir /path/to/notes --port 8090
```

## 文档

| 文档 | 内容 |
| --- | --- |
| [AGENTS.md](AGENTS.md) | 开发/协作入口：必做三件事 + 按需阅读索引 |
| [docs/agents/](docs/agents/) | 架构、数据约定、实现坑、测试、工作流 |
| [docs/noteschain-prd-v05.md](docs/noteschain-prd-v05.md) | 产品规格（含 v0.6 修订段） |
| [docs/api.md](docs/api.md) | HTTP 接口文档（鉴权、错误码、全部接口） |
| [docs/handover.md](docs/handover.md) | 当前状态、未完成事项、性能数据 |
| [docs/verify.md](docs/verify.md) | 人工验收清单（真实目录 / 云桌面） |
| [docs/release.md](docs/release.md) | 发布说明与操作手册 |
| [docs/perf-report.md](docs/perf-report.md) | 性能测法与结果 |

## 环境

- Windows：Rust stable（MSVC 工具链）+ VS 2022 Build Tools。
- Linux：Rust stable 即可（已在 Rust 1.94 / Ubuntu 上验证构建与运行）。
- 运行期建议装 git（用于版本历史）；不装则版本历史停用，其余功能正常。
- crates 走 rsproxy 镜像，配置见 `~/.cargo/config.toml`；无该配置时需要能访问 crates.io。

编辑器需要打包（产物已提交；改 `web/src/editor.js` 后才需要重建）：

```powershell
cd web; npm install; npm run build; cd ..
```
