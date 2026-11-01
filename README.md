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

- 首页：日程（本日 / 本周 / 本月）+ 全部待办，支持快速添加待办与日程
- 待办：勾选即按行回写原文件（抗外部插行）、`!p0`–`!p9` 优先级、排序与筛选
- 编辑：CodeMirror Live Preview，默认预览、点「编辑」进入修改；自动保存与冲突处理
- 搜索：子串匹配 + 标签/状态过滤 + 上下文；标签树多级折叠
- Git：自动 init、30 秒防抖提交、历史/diff、按文件回滚
- 安全：启动 token + Cookie、路径穿越防护

## 验证

```powershell
pwsh -File tools/release-gate.ps1
```

一条命令跑完构建、单元测试、clippy、接口冒烟、边界场景、平替预演与浏览器交互，全绿再提交。

## 文档

| 文档 | 内容 |
| --- | --- |
| [AGENTS.md](AGENTS.md) | 开发/协作入口：必做三件事 + 按需阅读索引 |
| [docs/agents/](docs/agents/) | 架构、数据约定、实现坑、测试、工作流 |
| [docs/noteschain-prd-v05.md](docs/noteschain-prd-v05.md) | 产品规格（含 v0.6 修订段） |
| [docs/handover.md](docs/handover.md) | 当前状态、未完成事项、性能数据 |
| [docs/verify.md](docs/verify.md) | 人工验收清单（真实目录 / 云桌面） |
| [docs/release.md](docs/release.md) | 发布说明与操作手册 |
| [docs/perf-report.md](docs/perf-report.md) | 性能测法与结果 |

## 环境

Rust stable（MSVC 工具链）+ VS 2022 Build Tools；运行期建议装 git（用于版本历史）。
crates 走 rsproxy 镜像，配置见 `~/.cargo/config.toml`。

编辑器需要打包（产物已提交；改 `web/src/editor.js` 后才需要重建）：

```powershell
cd web; npm install; npm run build; cd ..
```
