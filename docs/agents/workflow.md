# 工作流与风格

## 启动

```powershell
.\target\release\noteschain.exe --dir "D:\MyNotes" --port 8090
```

| 参数 | 说明 |
| --- | --- |
| `--dir` | 项目根（必填） |
| `--port` | 默认 **8090**，只绑 127.0.0.1 |
| `--links` | 启用双链 `[[目标]]`（默认关闭） |
| `--no-git` | 关闭 Git 自动提交，其余功能照常 |
| `--no-token` | 禁用 token 鉴权（仅建议个人独占机器；默认仍校验） |
| `--commit-debounce` | Git 自动提交防抖秒数，默认 **300**（停止变更 5 分钟合并提交） |
| `--commit-force` | 连续编辑强制提交秒数，默认 **1800** |

启动会打印带 token 的地址；token 每次启动随机生成，**重启即失效**。除进程内存外，
CLI 会在用户级目录的 `instances.json` 存一份（权限 600），用于免手工传 token；
该登记表绝不落在笔记目录内，否则会被 Git 自动提交收走。

> 端口提示：本机 8080 常被 Steam 的远程调试端口占用，所以默认改成了 8090。

## 定向写入 CLI

服务运行时另开终端执行 `noteschain add todo|schedule|line --text ...`，参数见根目录 `README.md`。要点：

- CLI 走 HTTP 调本机 server，**server 必须先起来**；找不到实例时报错退出，不自动拉起、不自动重试。
- 实例登记表在用户级目录（`$XDG_RUNTIME_DIR/noteschain/instances.json` 或 `%LOCALAPPDATA%\NotesChain\instances.json`），权限 600。**token 绝不写进笔记目录**（默认 `.gitignore` 不含 `.noteschain/`，会被自动提交收走）。
- CLI 显式传 `create: false`：文件不存在即拒绝。旧前端不传该字段，服务端仍按模板自动新建，保持零改动。
- 写入链路：`resolve()` → 按文件锁 → `create_dir_all`（仅新建时）→ `atomic_write` → `index.upsert` + `git.notify()` + `revision.fetch_add`。

## 提交

- 一个改动一个提交；提交信息注明对应 issue 编号（`#16`）或 PRD 章节
- 修复类提交写**根因**，不写「修复 bug」
- 提交前 `git status`：只应有源码/文档。`.demo/`、`target/`、`web/node_modules` 已忽略
- 不要提交 `.demo/` 下的测试语料

## 代码风格

**Rust**
- 错误：接口层用 `ApiError`（固定错误码枚举，不要随意新增 `code` 字符串）；内部用 `anyhow`
- 新增接口：先想清楚要不要 `resolve()`、要不要按文件锁、要不要 `notify + revision`
- 注释用中文，解释「为什么」

**前端**
- 无框架、不引第三方运行时库；建 DOM 用 `el(tag, cls, text)`
- 颜色/间距走 `:root` 变量；暗色用 `prefers-color-scheme`
- 交互类改动先在浏览器里点一遍，再用 `browser-probe` 固化

## 文档

| 场景 | 改哪个 |
| --- | --- |
| 产品行为变化 | `docs/noteschain-prd-v05.md`（在 v0.6 段追加修订说明，不要直接改历史章节） |
| 新增/修复反馈 | `docs/issues-v0.2.md` 追加一行，标 ✅ |
| 架构/约定变化 | `docs/agents/*.md` 对应文件 |
| 状态变化 | `docs/handover.md`（只记状态、未完成、性能） |
| 新增/改 CLI 命令或接口字段 | `README.md` + `docs/api.md` |
