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

v0.1.3 已内置终端 CLI（`today` / `todo` / `search` / `schedule` / `note` / `git`）。以下命令在**服务运行时**另开终端执行，可指定文件与标题：

```bash
noteschain todo add "买牛奶 #生活" --file 2026-11-09 --section "## 待办"
noteschain note add "和财务确认口径" --file 周报 --section "## 记录"
noteschain schedule --add "15:00-16:00 周会 @会议室" --today
noteschain todo add "发周报" --dry-run --json
```

| 参数 | 说明 |
| --- | --- |
| `--file` | 目标文件：**可只写文件名**（如 `2026-11-09`、`周报`），按索引自动展开成完整路径；含 `/` 的完整路径照旧；同名多个文件会报错并列出候选路径，请改用完整路径。缺省今日日记 |
| `--today` | 显式指向今日日记，与 `--file` 互斥（省得猜系统日期） |
| `--section` | 目标标题，接受 `## 待办` 或 `待办`；todo 缺省 `## 待办`，schedule 缺省 `## 日程` / `## 今日日程`，note 不传则追加到文件尾（旧行为） |
| `--create` | 显式允许新建；不传时沿用 v0.1.3 行为（今日日记按模板新建，其他目标新建空文件） |
| `--unique` | 目标段内已有完全相同行时跳过 |
| `--dry-run` | 只预览，零写入 |
| `--json` | 全局标志，原样输出服务端返回体 |

三个 append 接口共用同一套字段与写顺序（短名解析 → `resolve()` → 按文件锁 → 原子写 → 索引 → `git.notify()` → `revision+1`），CLI 绝不直接读写笔记文件。短名唯一命中才自动展开，重名只提醒不静默选一个；零命中时日期形 `YYYY-MM-DD` 按目录模型展开、其余补 `.md` 走原有新建逻辑。日程的 `--section` 只允许 `## 日程` / `## 今日日程`，写到别处不会出现在日程视图。

不传任何新参数时，旧命令行为不变。退出码沿用 v0.1.3 约定：`0` 成功、`1` 其他、`2` 服务端未启动、`3` 鉴权失败、`4` 服务端业务错误。

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
