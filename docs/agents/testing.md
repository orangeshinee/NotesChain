# 测试与验证

## 一条命令

```powershell
pwsh -File tools/release-gate.ps1
```

依次跑：构建 → `cargo test` → `cargo clippy` → 接口冒烟 → 边界场景 → 平替预演 → 浏览器交互（逐场景）→ **Playwright e2e（可选层**，检测到 `playwright-cli` 才跑，否则记 SKIP）。输出汇总表，非零退出即失败。**提交前必须全绿。**

> 不要在任何文档里写死「N 个测试 / N 项检查」——增删测试就会过期，只写命令。

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

- **CDP 探针**（`tools/browser-probe.ps1`）：直接开 Edge `--headless=new` 走 CDP，轻量、已覆盖 16 个场景。
  局限：页面内 `setTimeout` 链在 headless 下可能被节流，`Runtime.evaluate(awaitPromise)` 会拿不到结果，
  涉及「等待若干秒」的时序用例不要放这里。
- **Playwright**（`tools/e2e-savetype.ps1`）：需要真实等待/连续输入的时序用例用 `playwright-cli`，
  `page.evaluate` 能正确 await 页面内 Promise。约定：脚本自己负责清理测试写入（如删掉追加的 Z）。

## 加测试的原则

- 解析/定位类改动：**先补单测**再改实现
- 前端交互改动：至少补一个 `browser-probe` 场景，或手动在浏览器验证并在提交信息里写清步骤
- 修 bug：提交信息写清**根因**（例：「点标签名触发跳转，stopPropagation 抢走了展开」），不要只写「修复标签树」
