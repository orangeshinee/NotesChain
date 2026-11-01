# 架构

## 模块

| 文件 | 职责 |
| --- | --- |
| `src/main.rs` | CLI 解析、启动日志、监听绑定 |
| `src/api.rs` | 全部 HTTP 接口、token 鉴权中间件、统一错误码、静态资源（/app.js /style.css /editor.js /api/asset） |
| `src/index.rs` | 内存索引：扫描、日记判定（路径优先 / 内容兜底）、版本号、`TodoRow` |
| `src/markdown.rs` | 行级解析：代码块围栏、待办、标签、完成标记剥离、`!pN` 优先级、`todo:` 迁移识别 |
| `src/todo_write.rs` | 三级定位（行号→全文唯一→就近→拒绝）、行改写、原子写（临时文件 + rename） |
| `src/schedule.rs` | 日程解析（`## 日程` / `## 今日日程` 都认）、未识别行、疑似日程 |
| `src/git.rs` | git 检测、自动 init、30 秒防抖提交、失败 3 次停用 |
| `src/watch.rs` | notify 监听 + 300ms 防抖 + 增量 upsert |
| `src/state.rs` | `root` / `notes_root` / `assets_root`、启动 token、修订号、按文件写锁、路径解析 `resolve()` |

前端（无框架、无构建，除非改编辑器）：

| 文件 | 职责 |
| --- | --- |
| `web/index.html` | 布局骨架（侧边栏 + 主区） |
| `web/app.js` | 路由（hash）、渲染器、各视图、编辑器接线、轮询 |
| `web/style.css` | 样式（含暗色模式、打印样式） |
| `web/src/editor.js` → `web/dist/editor.js` | CodeMirror 6 扩展，esbuild 打包后由 `include_str!` 内嵌 |

## 目录模型

```
MyNotes/                ← --dir 指向这里（项目根，git 仓库一般也在这里）
├── notes/              ← 所有 .md
│   └── 2026/11/2026-11-01.md
├── assets/             ← 图片附件
├── templates/          ← 可选：自定义模板
└── .trash/             ← 删除笔记的回收站（不扫描）
```

- 若 `<root>/notes` 不存在 → 兼容旧布局，把 `<root>` 当笔记目录
- **API 里的路径相对 notes 根**（`2026/11/2026-11-01.md`），不是相对项目根
- 日记判定：路径 `YYYY/MM/YYYY-MM-DD.md` 且年月一致 → 日记；否则看首个一级标题是否 `# YYYY-MM-DD`；都不是 → 普通笔记

## 写操作的固定顺序

任何改文件的接口都必须按这个顺序（`api.rs` 里已有例子）：

1. `st.resolve(rel)` 做规范化 + 根目录包含校验（不要自己拼路径）
2. `st.lock_for(&rel).await` 拿**按文件锁**，串行化同一文件的写
3. `todo_write::atomic_write()` 原子写（临时文件 + rename）
4. `st.index.write().await.upsert(&st.notes_root, &rel)` 更新索引
5. `st.git.notify()` 进入提交队列
6. `st.revision.fetch_add(1, SeqCst)` 让前端轮询感知变化

漏掉 4/5/6 的表现分别是：搜索搜不到、不产生提交、界面不刷新。

## 前端状态

`state`（`app.js` 顶部）：`activePath`、`activeTag`、`previewMode`（每个文件打开都复位为 true）、`scheduleRange`、`todoStatus`、`todoSort`、`todoTag`、`expandedTags`、`pathSet`、`duplicateNames`。

外部改动的发现靠轮询 `GET /api/revision`（5 秒）；修订号变化时刷新侧边栏，并且只有磁盘版本真的变了才重载当前文件。
