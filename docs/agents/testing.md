# 测试与验证

## 一条命令

```powershell
pwsh -File tools/release-gate.ps1
```

依次跑：构建 → `cargo test` → `cargo clippy` → 接口冒烟 → 边界场景 → 平替预演 → 浏览器交互，输出汇总表，非零退出即失败。**提交前必须全绿。**

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

## 浏览器探针用法

```powershell
# 先起服务（拿到 token 地址），再跑
pwsh -File tools/browser-probe.ps1 -Url "http://127.0.0.1:8090/?token=..." -Hash "#/file/2026/11/2026-11-01.md" -Scenario all
```

- 需要编辑态的场景会自动点「编辑」（默认是预览模式）
- 场景列表见脚本 `ValidateSet`；新增场景时记得同步 `release-gate` 里对关键字段的断言（否则会「假绿」）

## 加测试的原则

- 解析/定位类改动：**先补单测**再改实现
- 前端交互改动：至少补一个 `browser-probe` 场景，或手动在浏览器验证并在提交信息里写清步骤
- 修 bug：提交信息写清**根因**（例：「点标签名触发跳转，stopPropagation 抢走了展开」），不要只写「修复标签树」
