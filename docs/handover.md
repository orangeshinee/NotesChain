# 交接文档

日期：2026-11-01  
适用版本：当前 `master` 最新提交

## 一句话

NotesChain 是一个**单文件 Rust 后端 + 无构建依赖前端**的本地 Markdown 笔记工具：每天一个文件、勾选即回写、改完自动进 Git 版本链。PRD 见 [noteschain-prd-v05.md](noteschain-prd-v05.md)，逐条实现状态见 [prd-audit.md](prd-audit.md)。

## 怎么跑起来

```powershell
cargo build --release
.\target\release\noteschain.exe --dir D:\notes --port 8080
```

终端会打印带 token 的地址。参数：

| 参数 | 作用 |
| --- | --- |
| `--dir` | 笔记根目录（必填） |
| `--port` | 端口，默认 8080，只绑 127.0.0.1 |
| `--links` | 开启双链 `[[目标]]`（实验性，默认关） |
| `--no-git` | 关闭 Git 自动提交，其余功能照常 |

## 代码地图

| 文件 | 职责 | 关键点 |
| --- | --- | --- |
| `src/main.rs` | CLI 与启动 | 参数、启动日志、监听绑定 |
| `src/api.rs` | 全部 HTTP 接口 | 鉴权中间件、路径防护、错误码、静态资源 |
| `src/index.rs` | 内存索引 | 扫描、日记判定（路径优先/内容兜底）、版本号、标签集合 |
| `src/markdown.rs` | 行级解析 | 代码块围栏状态机、待办、标签、完成标记剥离、`todo:` 识别 |
| `src/todo_write.rs` | 回写与原子写 | 三级定位、行改写、临时文件 + rename |
| `src/schedule.rs` | 日程解析 | `## 今日日程` 段、未识别行、疑似日程 |
| `src/git.rs` | Git 调度 | 检测、自动 init、30s 防抖、失败 3 次停用 |
| `src/watch.rs` | 文件监听 | notify + 300ms 防抖 + 增量索引 |
| `src/state.rs` | 应用状态 | 根目录、token、修订号、按文件写锁、路径解析 |
| `web/index.html` | 页面骨架 | 侧边栏 + 主区域 |
| `web/app.js` | 前端逻辑 | 路由、渲染器、搜索、编辑器接线、各视图 |
| `web/style.css` | 样式 | 含暗色模式与打印样式 |
| `web/src/editor.js` | CodeMirror 6 | Live Preview 装饰、待办复选框、图片/表格/链接、悬浮预览 |

## 几个必须知道的实现约束

1. **块级装饰只能来自 StateField**：`Decoration.replace({block:true})` 不能由 ViewPlugin 提供，表格渲染因此走 StateField。
2. **装饰区间必须有序不重叠**：否则 RangeSetBuilder 抛错，整页装饰失效。
3. **不要给隐藏的 CodeMirror setValue**：`display:none` 容器里 CM 会陷入测量死循环、整页卡死（预览因此保持只读）。
4. **Windows 路径**：`canonicalize` 返回 `\\?\F:\...`，git 不认，统一用 `state::simplify` 去掉前缀。
5. **编码**：读取时剥 UTF-8 BOM；保存与行回写按原 `eol` 还原 CRLF，避免整篇 diff。
6. **路径防护**：所有接口路径以 notes 根为基准做规范化 + 包含校验，`..`/绝对路径/软链跳出一律 400。

## 验证体系（一条命令）

```powershell
pwsh -File tools/release-gate.ps1
```

19 项检查：构建、25 个单测、clippy、接口冒烟 18 项、边界 18 项、平替预演 20 项、13 项真实浏览器交互（CDP 驱动）。

单独跑：

| 脚本 | 作用 |
| --- | --- |
| `tools/smoke.ps1` | 接口层冒烟（鉴权/扫描/日程/标签/搜索/回写/冲突/资源/模板） |
| `tools/edge-check.ps1` | 脏数据：怪文件名、1MB 文件、GBK、深层目录、.gitignore、空目录 |
| `tools/legacy-check.ps1` | 自造 Silver Bullet 旧目录的平替预演（含「已有文件零改写」哈希比对） |
| `tools/browser-probe.ps1` | 真实浏览器交互：悬停/勾选/自动保存/冲突/搜索跳转/标签/预览/虚拟列表/快速添加/导出/列宽 |
| `tools/verify-real-dir.ps1` | **对真实目录只读体检**（`--no-git`，不建仓库不改文件） |
| `tools/gen-corpus.js` | 生成 1000 文件性能语料 |

## 性能

见 [perf-report.md](perf-report.md)：1000 文件 / 18.8 MB 语料下，冷启动 0.30s、空载 29 MB、待办 104 ms、搜索 8–18 ms。

两个已知边界：
- `/api/todos` 仍是 O(n) 串行化，数万条待办会变慢（已有 limit/offset 与「显示全部」）
- 索引常驻全文，内存约为语料 1.5 倍

## 仍需在真实环境完成的

1. **真实笔记目录平替验证**：`pwsh -File tools/verify-real-dir.ps1 -Path "D:\你的目录" -Out .\体检报告.md`
2. **云桌面性能复测**：按 `docs/verify.md` 第 7 步，记录机器配置与网络盘行为
3. **应用内 Git 提交复核**：开发机沙箱写不进 `.git`，需在普通终端确认 30 秒内产生 `auto: ...` 提交

## 继续开发时的建议顺序

1. 先跑 `release-gate`，确认基线是绿的
2. 改前端交互类代码后，务必用 `browser-probe` 验证（历史上「换实现后旧交互静默失效」踩过两次）
3. 改解析/回写逻辑后，补对应单测并跑 `smoke`
4. 提交前看 `git status`，只提交源码与文档，`.demo/` 已在 .gitignore 里
