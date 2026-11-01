# AGENTS.md

给 AI 编码代理（以及新加入的人）的项目说明书。**先读这个，再动代码。**

## 这是什么

NotesChain —— 极度轻量、本地优先的 Markdown 笔记工具。Rust（Axum）后端 + 原生 JS 前端（CodeMirror 6 只用于编辑器），编译成单个 exe，前端资源编译期内嵌。

产品规格：[docs/noteschain-prd-v05.md](docs/noteschain-prd-v05.md)（含 v0.6 修订段）｜现状与坑：[docs/handover.md](docs/handover.md)｜待办与已修：[docs/issues-v0.2.md](docs/issues-v0.2.md)

## 常用命令

```powershell
cargo build --release            # 产物 target\release\noteschain.exe
cargo test                       # 26 个单元测试
cargo clippy --all-targets       # 必须零警告

# 改了 web/src/editor.js 之后必须重新打包（否则内嵌的是旧包！）
cd web; npm install; npm run build; cd ..

pwsh -File tools\release-gate.ps1   # 一条命令跑完全部验证（22 项），提交前必跑
```

运行：

```powershell
.\target\release\noteschain.exe --dir "D:\MyNotes" --port 8090
```

参数：`--dir`（项目根，必填）、`--port`（默认 8080）、`--links`（启用双链）、`--no-git`（关闭自动提交）。
启动会打印带 token 的地址，token 每次启动都变。

## 目录结构

```
src/           Rust 后端
  main.rs      CLI、启动
  api.rs       所有 HTTP 接口、鉴权、错误码、静态资源
  index.rs     内存索引、日记判定、版本号、TodoRow
  markdown.rs  行级解析：围栏、待办、标签、完成标记、!pN 优先级、todo: 识别
  todo_write.rs 三级定位、行改写、原子写
  schedule.rs  日程解析（## 日程 / ## 今日日程）
  git.rs       检测、自动 init、30s 防抖提交
  watch.rs     notify 监听 + 增量索引
  state.rs     root / notes_root / assets_root、token、修订号、按文件锁
web/           前端（手写，无构建）
  index.html app.js style.css
  src/editor.js   CodeMirror 6 扩展（Live Preview、复选框、表格、悬浮预览）
  dist/editor.js  esbuild 产物，**编译期内嵌，改源码后必须重建**
docs/          PRD、验收表、性能报告、交接、发布说明、模板
tools/         验证脚本（release-gate / smoke / edge-check / legacy-check / browser-probe / gen-corpus / verify-real-dir）
```

## 目录模型（重要）

- `--dir` 指向**项目根**，例如 `D:\MyNotes`
- Markdown 在 `D:\MyNotes\notes\**`，附件在 `D:\MyNotes\assets\**`
- 兼容旧布局：若 `<root>/notes` 不存在，则把 `<root>` 当笔记目录
- 日记路径：`notes/YYYY/MM/YYYY-MM-DD.md`；API 里的路径**相对 notes 根**（`2026/11/2026-11-01.md`）
- 删除笔记是移到 `<root>/.trash/时间戳-文件名`，不硬删

## 数据约定

- **绝不改写已有笔记文件**（PRD 红线）。新建才写模板，读取不改内容。
- 模板：`<root>/templates/*.md`，占位符 `{{TITLE}}` `{{DATE}}` `{{DATETIME}}` `{{TAGS}}`；内置模板在 `docs/templates/`，用 `include_str!` 内嵌。
- frontmatter 四字段：`title / type / created / tags`；渲染时剥成「元数据卡片」，不进正文。
- 待办优先级语法：`!p0`–`!p9`（大小写不敏感，非法写法如 `!p10` 不识别）。**不要**与 `#标签` 混用。
- 待办回写靠 `raw_line`（去掉完成标记后的整行）逐字匹配，**改动解析时不要动 raw_line 的生成**。
- 编码：读取剥 UTF-8 BOM；保存与行回写按原 `eol` 还原 CRLF。非 UTF-8 文件只告警不写。

## 改代码时必须知道的坑

1. **`web/src/editor.js` 改完要 `npm run build`**，再 `cargo build`。只 cargo build 的话内嵌的还是旧 dist，功能不会生效。
2. **块级装饰只能来自 StateField**：`Decoration.replace({block:true})` 放在 ViewPlugin 里会抛错并让整页装饰失效（表格就这么做的）。
3. **装饰区间必须有序且不重叠**，否则 RangeSetBuilder 抛错。
4. **不要给隐藏的 CodeMirror `setValue`**：`display:none` 容器里会进入测量死循环、整页卡死（预览因此只读）。
5. **Windows 路径**：`canonicalize` 返回 `\\?\F:\...`，git 不认，统一用 `state::simplify`。
6. **API 路径校验**：一律经 `AppState::resolve`（规范化 + 根目录包含校验），不要自己拼接。
7. **写完文件要 `st.git.notify()` 并 `st.revision.fetch_add(1)`**，否则自动提交和前端轮询都不会收到变化。
8. **PowerShell 的 `-Path` 把 `[ ]` 当通配符**（写测试脚本时踩过），处理怪文件名用 `-LiteralPath` 或 .NET API。
9. **验证脚本自己也会腐坏**：改前端交互后务必跑 `tools/browser-probe.ps1`（历史上「换实现后旧交互静默失效」踩过两次）。

## 测试与验证

| 层 | 命令 | 覆盖 |
| --- | --- | --- |
| 单测 | `cargo test` | 解析、三级定位、日程、路径、编码、优先级 |
| 接口冒烟 | `pwsh -File tools/smoke.ps1` | 鉴权/扫描/日程/标签/搜索/回写/冲突/资源/模板 |
| 边界 | `pwsh -File tools/edge-check.ps1` | 怪文件名、1MB 文件、GBK、深层目录、.gitignore、空目录 |
| 平替预演 | `pwsh -File tools/legacy-check.ps1` | 自造 SB 风格旧目录 + 文件哈希不变 |
| 浏览器交互 | `pwsh -File tools/browser-probe.ps1 -Url <带 token 地址> -Hash "#/file/..." -Scenario all` | 悬停/勾选/自动保存/冲突/搜索跳转/标签/预览/虚拟列表/快速添加/导出/列宽/滚动 |
| 真实目录体检 | `pwsh -File tools/verify-real-dir.ps1 -Path "D:\MyNotes"` | 只读，不建仓库不改文件 |

提交前：`pwsh -File tools/release-gate.ps1`（22 项），全绿再 commit。

## 提交习惯

- 一个改动一个提交，提交信息注明对应 issue 编号（`#12`）或 PRD 章节
- 提交前看 `git status`：只应有源码/文档；`.demo/`、`target/`、`web/node_modules` 已被忽略
- 不要 `--no-verify`，不要提交 `.demo/` 下的测试语料

## 代码风格

- Rust：`cargo fmt` 默认风格；错误用 `anyhow`/`ApiError`，接口错误码固定枚举（见 `docs/prd-audit.md` 7.3.1）
- 前端：原生 JS，无框架；新增 DOM 用 `el(tag, cls, text)` 帮助函数，不引第三方库
- CSS：颜色/间距走 `:root` 变量；暗色模式用 `prefers-color-scheme`
- 中文注释可以，注释解释「为什么」而不是「是什么」
