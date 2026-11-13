# 测试与验证

## 按平台选闸门

`tools/release-gate.ps1` 是 Windows 版（硬编码 `target\release\noteschain.exe`、`F:\Dev\Packages\nodejs` 等路径），Linux 上装了 pwsh 也跑不通。所以两个平台各有一条命令，全绿再提交：

| 平台 | 命令 | 覆盖 |
| --- | --- | --- |
| Windows | `pwsh -File tools/release-gate.ps1` | 构建 → 单元测试 → clippy → 接口冒烟（鉴权 / 扫描 / 回写 / 冲突）→ 边界场景 → 平替预演 → 浏览器交互 → Playwright e2e（可选） |
| Linux / macOS | `bash tools/release-gate.sh` | 前端静态检查 → 构建 → 单元测试 → clippy `-D warnings` → 二进制可用性 → **只读接口检查**（`/api/files` / `/api/config` / 首页） |

Linux 脚本的构建 / 测试 / clippy / 二进制四步与 `.github/workflows/ci.yml` 的 `check` job 一致，是跨平台的最小契约。服务检查在隔离临时目录起一次真实进程，用唯一哨兵文件断言响应确实来自本次启动的二进制（否则端口被占用会把别的实例当成被测对象，造成假绿），**不碰真实笔记库**，跑完自动清理。

**覆盖差异，别把 Linux 全绿当成 Windows 全绿**：
- Linux 的「只读接口检查」是几个 `--no-token` 的 GET，不覆盖鉴权、扫描、回写、版本冲突；Windows 的 `smoke.ps1` 是带 token 的写操作冒烟。两者同名不同义，勿等同。
- ps1 里的边界场景、平替预演、浏览器交互探针、`browser-probe.ps1` 场景在 Linux 版里都不跑。Linux 闸门**没有任何浏览器层**，前端交互改动还需另用浏览器脚本或手工验证，并在提交信息里写明步骤。
- 这些改动若涉及扫描/索引/文件判定/前端交互，还需在 Windows 侧补跑。

依次跑（Windows 版）：构建 → `cargo test` → `cargo clippy` → 接口冒烟 → 边界场景 → 平替预演 → 浏览器交互（逐场景）→ **Playwright e2e（可选层**，检测到 `playwright-cli` 才跑，否则记 SKIP）。输出汇总表，非零退出即失败。**提交前必须全绿。**

> 不要在任何文档里写死「N 个测试 / N 项检查」——增删测试就会过期，只写命令。
> 闸门自身也要验：改动脚本后做一次负向测试（故意注入一个语法错误），确认它真的会红、会返回非零退出码，避免「假绿」。

## 分层

| 层 | 脚本 | 覆盖 | 什么时候必须跑 |
| --- | --- | --- | --- |
| 单元测试 | `cargo test` | 解析、三级定位、日程、路径防护、编码、优先级 | 改 `markdown/index/todo_write/schedule` 后 |
| 接口冒烟 | `tools/smoke.ps1` | 鉴权、扫描、日程、标签、搜索、回写、冲突、资源、模板 | 改 `api.rs` 后 |
| 边界场景 | `tools/edge-check.ps1` | 怪文件名、1MB 文件、GBK、深层目录、.gitignore、空目录 | 改扫描/索引后 |
| 平替预演 | `tools/legacy-check.ps1` | 自造 SB 风格旧目录 + 前后 SHA256 比对（确保零改写） | 改文件判定/解析后 |
| 浏览器交互 | `tools/browser-probe.ps1` | 悬停、勾选、自动保存、冲突、搜索跳转、标签展开、预览、虚拟列表、快速添加、导出、列宽、滚动保持 | **改前端交互后必跑** |
| 真实目录体检 | `tools/verify-real-dir.ps1 -Path <目录>` | 只读：不建仓库、不改文件，输出识别结果 | 交付前、或用户报「识别不对」时 |
| 保存时序 e2e | `tools/e2e-savetype.ps1 -Url <带 token 地址>` | Playwright 驱动：保存中继续输入，断言最终磁盘 == 编辑器 | 改保存状态机后必跑 |

## 浏览器探针用法

```powershell
# 先起服务（拿到 token 地址），再跑
pwsh -File tools/browser-probe.ps1 -Url "http://127.0.0.1:8090/?token=..." -Hash "#/file/2026/11/2026-11-01.md" -Scenario all
```

- 需要编辑态的场景会自动点「编辑」（默认是预览模式）
- 场景列表见脚本 `ValidateSet`；新增场景时记得同步 `release-gate` 里对关键字段的断言（否则会「假绿」）

## 浏览器层的两种驱动

- **CDP 探针**（`tools/browser-probe.ps1`）：直接开 Edge `--headless=new` 走 CDP，轻量。
  局限：页面内 `setTimeout` 链在 headless 下可能被节流，`Runtime.evaluate(awaitPromise)` 会拿不到结果，
  涉及「等待若干秒」的时序用例不要放这里。
- **Playwright**（`tools/e2e-savetype.ps1`）：需要真实等待/连续输入的时序用例用 `playwright-cli`，
  `page.evaluate` 能正确 await 页面内 Promise。约定：脚本自己负责清理测试写入（如删掉追加的 Z）。

## 加测试的原则

- 解析/定位类改动：**先补单测**再改实现
- 前端交互改动：至少补一个 `browser-probe` 场景，或手动在浏览器验证并在提交信息里写清步骤
- 修 bug：提交信息写清**根因**（例：「点标签名触发跳转，stopPropagation 抢走了展开」），不要只写「修复标签树」
