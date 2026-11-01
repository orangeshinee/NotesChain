# AGENTS.md

NotesChain：单文件 Rust（Axum）+ 原生 JS 的本地 Markdown 笔记工具，前端资源编译期内嵌进二进制。

## 每次任务都要做的三件事

1. **构建**：`cargo build --release`
   - 若改了 `web/src/editor.js`，必须先 `cd web; npm run build; cd ..`，否则内嵌的是旧包
2. **验证**：`pwsh -File tools/release-gate.ps1`，全绿再提交
3. **红线**：**绝不改写已有笔记文件**；只有新建文件才写模板

## 按需阅读

| 文档 | 什么时候读 |
| --- | --- |
| [docs/agents/architecture.md](docs/agents/architecture.md) | 改后端、路由、索引、目录模型 |
| [docs/agents/data-conventions.md](docs/agents/data-conventions.md) | 碰解析、模板、frontmatter、优先级、编码 |
| [docs/agents/pitfalls.md](docs/agents/pitfalls.md) | **动代码前必扫**，全是踩过的坑 |
| [docs/agents/testing.md](docs/agents/testing.md) | 加/改测试、判断覆盖够不够 |
| [docs/agents/workflow.md](docs/agents/workflow.md) | 启动参数、端口、提交习惯、代码风格 |
| [docs/handover.md](docs/handover.md) | 做到哪了、还差什么、性能数据 |
| [docs/noteschain-prd-v05.md](docs/noteschain-prd-v05.md) | 产品行为以它为准（含 v0.6 修订段） |
